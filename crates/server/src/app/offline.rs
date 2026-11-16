//! Offline replay for network-backed tools.
//!
//! Each tool response is stored in the tome research cache (it already holds
//! a creation timestamp) so that, when `skrills_state::is_offline()` holds,
//! the newest stored answer can be served with its age instead of failing on
//! a request that cannot succeed.

use std::future::Future;

use anyhow::{anyhow, Result};
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{json, Value};
use skrills_state::cache::human_age;
use skrills_tome::cache::ResearchCache;

use crate::mcp_result::tool_ok;

/// Literal text that marks a response served from cache.
pub(crate) const OFFLINE_NOTICE: &str = "offline \u{2014} using cached data";

/// A stored response and when it was stored.
#[derive(Debug)]
pub(crate) struct Replay {
    pub(crate) value: Value,
    pub(crate) cached_at_unix: u64,
}

impl Replay {
    /// One line saying the data is cached and how old it is.
    pub(crate) fn notice(&self) -> String {
        format!(
            "{OFFLINE_NOTICE} (cached {})",
            human_age(self.cached_at_unix)
        )
    }
}

/// Result of [`fetch_or_replay`].
#[derive(Debug)]
pub(crate) enum Outcome {
    Live(Value),
    Cached { value: Value, notice: String },
}

fn cache_key(api: &str, key: &str) -> String {
    format!("{api}:{key}")
}

/// Opens the shared cache. A cache that cannot be opened must not break
/// online use, so the failure is logged and callers carry on without one.
pub(crate) fn open_cache() -> Option<ResearchCache> {
    match ResearchCache::open() {
        Ok(cache) => Some(cache),
        Err(e) => {
            tracing::warn!(error = %e, "research cache unavailable; responses will not be kept for offline use");
            None
        }
    }
}

/// Stores `value` as the latest answer for `api`/`key`. Best effort: a failed
/// write only costs a later offline hit.
pub(crate) fn remember(
    cache: Option<&ResearchCache>,
    api: &str,
    key: &str,
    query: &str,
    value: &Value,
) {
    let Some(cache) = cache else { return };
    if let Err(e) = cache.put(&cache_key(api, key), api, query, &value.to_string(), None) {
        tracing::warn!(api, error = %e, "could not store response for offline use");
    }
}

/// The newest stored answer for `api`/`key`, or an error that names offline
/// mode so the caller knows why no request was made.
pub(crate) fn replay(cache: Option<&ResearchCache>, api: &str, key: &str) -> Result<Replay> {
    let stored = match cache {
        Some(cache) => cache.get_latest(&cache_key(api, key))?,
        None => None,
    };
    let hit = stored.ok_or_else(|| {
        anyhow!(
            "offline mode: no cached data for {api} \"{key}\"; \
             go online or unset SKRILLS_OFFLINE to fetch it"
        )
    })?;
    Ok(Replay {
        value: serde_json::from_str(&hit.response_json)?,
        cached_at_unix: hit.cached_at_unix,
    })
}

/// Runs `fetch` and stores its answer, or when `offline` replays the stored
/// answer without calling `fetch`.
pub(crate) async fn fetch_or_replay<F, Fut>(
    cache: Option<&ResearchCache>,
    offline: bool,
    api: &str,
    key: &str,
    query: &str,
    fetch: F,
) -> Result<Outcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Value>>,
{
    if offline {
        let hit = replay(cache, api, key)?;
        let notice = hit.notice();
        return Ok(Outcome::Cached {
            value: hit.value,
            notice,
        });
    }
    let value = fetch().await?;
    remember(cache, api, key, query, &value);
    Ok(Outcome::Live(value))
}

/// A tool result for a cached `value`: the notice leads the text, and the
/// structured payload gains `offline` and `cached_at_unix` so a client can
/// tell replayed data from a live answer.
pub(crate) fn cached_tool_result(
    hit: Replay,
    summary: impl FnOnce(&Value) -> String,
) -> CallToolResult {
    let text = format!("{}: {}", hit.notice(), summary(&hit.value));
    let mut structured = hit.value;
    if let Some(fields) = structured.as_object_mut() {
        fields.insert("offline".into(), json!(true));
        fields.insert("cached_at_unix".into(), json!(hit.cached_at_unix));
    }
    tool_ok(vec![ContentBlock::text(text)], Some(structured))
}

/// Renders an [`Outcome`]: live data as it is, cached data with the notice.
pub(crate) fn outcome_to_tool_result(
    outcome: Outcome,
    summary: impl FnOnce(&Value) -> String,
) -> CallToolResult {
    match outcome {
        Outcome::Live(value) => tool_ok(vec![ContentBlock::text(summary(&value))], Some(value)),
        Outcome::Cached { mut value, notice } => {
            let text = format!("{notice}: {}", summary(&value));
            if let Some(fields) = value.as_object_mut() {
                fields.insert("offline".into(), json!(true));
            }
            tool_ok(vec![ContentBlock::text(text)], Some(value))
        }
    }
}
