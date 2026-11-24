//! Offline replay for network-backed tools.
//!
//! Each tool response is stored in the tome research cache (it already holds
//! a creation timestamp) so that, when `skrills_state::offline_source()` says
//! the process is offline, the newest stored answer can be served with its
//! age instead of failing on a request that cannot succeed. A live request
//! that fails falls back to the stored answer the same way.

use std::future::Future;

use anyhow::{anyhow, Result};
use rmcp::model::{CallToolResult, ContentBlock};
use serde_json::{json, Value};
use skrills_state::cache::human_age;
use skrills_state::OfflineSource;
use skrills_tome::cache::ResearchCache;

use crate::handler::run_blocking;
use crate::mcp_result::tool_ok;

/// Literal text that marks a response served from cache.
pub(crate) const OFFLINE_NOTICE: &str = "offline \u{2014} using cached data";

/// A stored response and when it was stored.
#[derive(Debug)]
pub(crate) struct Replay {
    pub(crate) value: Value,
    pub(crate) cached_at_unix: u64,
    /// Why a live request was not used, when one was tried and failed. `None`
    /// means it was never tried because the process is offline.
    pub(crate) live_error: Option<String>,
}

impl Replay {
    /// One line saying the data is cached and how old it is.
    pub(crate) fn notice(&self) -> String {
        let age = human_age(self.cached_at_unix);
        match &self.live_error {
            None => format!("{OFFLINE_NOTICE} (cached {age})"),
            Some(error) => {
                format!("live request failed ({error}); using cached data (cached {age})")
            }
        }
    }
}

/// Result of [`fetch_or_replay`].
#[derive(Debug)]
pub(crate) enum Outcome {
    Live(Value),
    Cached(Replay),
}

/// A live answer, and whether it may replace the stored one.
#[derive(Debug)]
pub(crate) enum Fresh {
    /// Every part of the answer was fetched; it is stored for offline use.
    Complete(Value),
    /// Part of the answer is missing because a request failed. It is
    /// returned but not stored, so a transient failure is not replayed later
    /// as if it were the answer.
    Partial(Value),
}

fn cache_key(api: &str, key: &str) -> String {
    format!("{api}:{key}")
}

/// The cache key part for a free-text `query` and result `limit`. Every tool
/// that keys by query builds it here, so the key a tool writes is the key it
/// reads back.
pub(crate) fn query_key(query: &str, limit: usize) -> String {
    format!("{}|{limit}", query.trim().to_lowercase())
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

/// The cache and the offline decision for one tool call. Both block (a
/// SQLite open, and a connectivity probe of up to a few seconds), so they run
/// off the async workers.
pub(crate) fn cache_and_offline_source() -> (Option<ResearchCache>, Option<OfflineSource>) {
    run_blocking(|| (open_cache(), skrills_state::offline_source()))
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

fn stored(cache: &ResearchCache, api: &str, key: &str) -> Result<Option<Replay>> {
    let Some(hit) = cache.get_latest(&cache_key(api, key))? else {
        return Ok(None);
    };
    Ok(Some(Replay {
        value: serde_json::from_str(&hit.response_json)?,
        cached_at_unix: hit.cached_at_unix,
        live_error: None,
    }))
}

/// The stored answer for `api`/`key`, if one can be read; a read failure is
/// logged and counts as none.
pub(crate) fn stored_value(cache: Option<&ResearchCache>, api: &str, key: &str) -> Option<Value> {
    match stored(cache?, api, key) {
        Ok(found) => found.map(|hit| hit.value),
        Err(e) => {
            tracing::warn!(api, error = %e, "could not read the cached answer");
            None
        }
    }
}

/// The newest stored answer for `api`/`key`, or an error that names offline
/// mode and what turned it on, so the caller knows why no request was made.
pub(crate) fn replay(
    cache: Option<&ResearchCache>,
    source: OfflineSource,
    api: &str,
    key: &str,
) -> Result<Replay> {
    let Some(cache) = cache else {
        return Err(anyhow!(
            "offline mode ({source}): the research cache could not be opened, \
             so no cached answer for {api} \"{key}\" can be read"
        ));
    };
    stored(cache, api, key)?.ok_or_else(|| {
        anyhow!(
            "offline mode ({source}): no cached data for {api} \"{key}\"; \
             go online to fetch it"
        )
    })
}

/// The stored answer to serve after a live request failed with `error`, or
/// `None` when there is none and `error` should be returned instead.
pub(crate) fn fallback(
    cache: Option<&ResearchCache>,
    api: &str,
    key: &str,
    error: &anyhow::Error,
) -> Option<Replay> {
    let found = match stored(cache?, api, key) {
        Ok(found) => found,
        Err(e) => {
            tracing::warn!(api, error = %e, "could not read the cached answer after a failed request");
            None
        }
    };
    found.map(|hit| Replay {
        live_error: Some(format!("{error:#}")),
        ..hit
    })
}

/// Runs `fetch` and stores a complete answer, or when `offline` is set
/// replays the stored answer without calling `fetch`. A failed `fetch`
/// falls back to the stored answer when there is one.
pub(crate) async fn fetch_or_replay<F, Fut>(
    cache: Option<&ResearchCache>,
    offline: Option<OfflineSource>,
    api: &str,
    key: &str,
    query: &str,
    fetch: F,
) -> Result<Outcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<Fresh>>,
{
    if let Some(source) = offline {
        return Ok(Outcome::Cached(replay(cache, source, api, key)?));
    }
    match fetch().await {
        Ok(Fresh::Complete(value)) => {
            remember(cache, api, key, query, &value);
            Ok(Outcome::Live(value))
        }
        Ok(Fresh::Partial(value)) => Ok(Outcome::Live(value)),
        Err(e) => match fallback(cache, api, key, &e) {
            Some(hit) => Ok(Outcome::Cached(hit)),
            None => Err(e),
        },
    }
}

/// A tool result for a cached `value`: the notice leads the text, and the
/// structured payload gains `offline` and `cached_at_unix` (and `live_error`
/// when a live request failed) so a client can tell replayed data from a
/// live answer.
pub(crate) fn cached_tool_result(
    hit: Replay,
    summary: impl FnOnce(&Value) -> String,
) -> CallToolResult {
    let text = format!("{}: {}", hit.notice(), summary(&hit.value));
    let mut structured = hit.value;
    if let Some(fields) = structured.as_object_mut() {
        fields.insert("offline".into(), json!(hit.live_error.is_none()));
        fields.insert("cached_at_unix".into(), json!(hit.cached_at_unix));
        if let Some(error) = hit.live_error {
            fields.insert("live_error".into(), json!(error));
        }
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
        Outcome::Cached(hit) => cached_tool_result(hit, summary),
    }
}
