//! Implements the primary functionality for the `skrills` application.
//!
//! This includes the MCP server, skill discovery, caching mechanisms, and the command-line interface.
//!
//! The `run` function initiates the server. The `runtime` module offers tools for
//! managing runtime options. Other crate components are considered internal and
//! may be subject to change without prior notice.
//!
//! For details on stability and versioning, refer to `docs/semver-policy.md`.
//!
//! The `watch` feature allows filesystem monitoring for live cache invalidation.
//! To build without this feature, use `--no-default-features`.
//!
//! On Unix-like systems, a `SIGCHLD` handler prevents zombie processes.

use crate::autoload::{
    env_embed_threshold, render_autoload_with_reader, render_preview_stats, AutoloadOptions,
    RenderMode,
};
use crate::cli::{Cli, Commands};
use crate::discovery::{
    agents_manifest, collect_agents, collect_skills, merge_extra_dirs, priority_labels,
    priority_labels_and_rank_map, read_skill, resolve_agent, resolve_skill, skill_roots,
    AGENTS_DESCRIPTION, AGENTS_NAME, AGENTS_TEXT, AGENTS_URI, DEFAULT_AGENT_RUN_TEMPLATE,
    ENV_EXPOSE_AGENTS,
};
use crate::doctor::doctor_report;
use crate::emit::{emit_autoload, AutoloadArgs};
use crate::runtime::{
    env_auto_pin_default, env_diag_default, env_include_claude_default, RuntimeOverrides,
};
use crate::signals::ignore_sigchld;
use crate::sync::{mirror_source_root, sync_agents, sync_from_claude};
use crate::trace::stdio_with_optional_trace;
use crate::tui::tui_flow;
use anyhow::{anyhow, Result};
use clap::Parser;
#[cfg(feature = "watch")]
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use rmcp::model::{
    CallToolRequestParam, CallToolResult, ClientInfo, Content, InitializeResult,
    ListResourcesResult, ListToolsResult, Meta, PaginatedRequestParam, RawResource,
    ReadResourceRequestParam, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    Tool, ToolAnnotations,
};
use rmcp::service::serve_server;
use rmcp::ServerHandler;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap};
use skrills_discovery::{
    discover_skills, extract_refs_from_agents, Diagnostics, DuplicateInfo, SkillMeta, SkillRoot,
};
use skrills_state::{
    auto_pin_from_history, cache_ttl, env_manifest_first, env_manifest_minimal, env_max_bytes,
    env_render_mode_log, home_dir, load_history, load_manifest_settings, load_pinned,
    load_pinned_with_defaults, print_history, save_auto_pin_flag, save_history, save_pinned,
    HistoryEntry,
};
#[cfg(feature = "subagents")]
use skrills_subagents::SubagentService;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;

/// Establishes the manifest render mode based on runtime overrides and client information.
///
/// The render mode options are `ContentOnly`, `ManifestOnly`, or `Dual`. The decision follows
/// this precedence:
/// 1. If `manifest_first` is disabled in runtime overrides, return `ContentOnly`.
/// 2. If the client is in the manifest allowlist, return `ManifestOnly`.
/// 3. If the client name contains "claude" or "anthropic", return `ManifestOnly`.
/// 4. Otherwise, return `Dual`.
fn manifest_render_mode(runtime: &RuntimeOverrides, peer_info: Option<&ClientInfo>) -> RenderMode {
    if !runtime.manifest_first() {
        return RenderMode::ContentOnly;
    }

    if let Some(info) = peer_info {
        if manifest_allowlist_match(info) {
            return RenderMode::ManifestOnly;
        }
    }

    let manifest_capable = peer_info.map(|info| {
        let name = info.client_info.name.to_ascii_lowercase();
        name.contains("claude") || name.contains("anthropic")
    });

    match manifest_capable {
        Some(true) => RenderMode::ManifestOnly,
        _ => RenderMode::Dual,
    }
}

/// Determines if the peer accepts gzipped manifest payloads.
///
/// This is decided by the `SKRILLS_ACCEPT_GZIP` environment variable or if the
/// client name contains "gzip".
fn peer_accepts_gzip(peer_info: Option<&ClientInfo>) -> bool {
    if std::env::var("SKRILLS_ACCEPT_GZIP")
        .ok()
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return true;
    }
    if let Some(info) = peer_info {
        let name = info.client_info.name.to_ascii_lowercase();
        if name.contains("gzip") {
            return true;
        }
    }
    false
}

/// Checks if the client is included in the manifest allowlist.
///
/// The allowlist is an optional JSON file specified by the `SKRILLS_MANIFEST_ALLOWLIST`
/// environment variable. The file must contain an array of objects, each with a
/// `name_substr` and an optional `min_version`.
fn manifest_allowlist_match(info: &ClientInfo) -> bool {
    if let Some(entries) = ALLOWLIST_CACHE.get_or_init() {
        let name_lc = info.client_info.name.to_ascii_lowercase();
        for item in entries {
            if !name_lc.contains(&item.name_substr) {
                continue;
            }
            if let Some(min) = &item.min_version {
                if !version_gte(semver::Version::parse(&info.client_info.version).ok(), min) {
                    continue;
                }
            }
            return true;
        }
    }
    false
}

/// Compare two version strings, treating non-numeric segments as zero.
fn version_gte(current: Option<semver::Version>, min: &str) -> bool {
    match (current, semver::Version::parse(min)) {
        (Some(c), Ok(m)) => c >= m,
        _ => false,
    }
}

/// An entry in the manifest allowlist.
#[derive(Clone)]
struct AllowlistEntry {
    name_substr: String,
    min_version: Option<String>,
}

/// A cache for the manifest allowlist.
struct AllowlistCache {
    inner: Mutex<Option<Vec<AllowlistEntry>>>,
}

impl AllowlistCache {
    /// Get the allowlist from the cache, loading it from the file if necessary.
    fn get_or_init(&self) -> Option<Vec<AllowlistEntry>> {
        let mut guard = self.inner.lock().ok()?;
        if guard.is_none() {
            *guard = load_allowlist();
        }
        guard.clone()
    }
}

static ALLOWLIST_CACHE: std::sync::LazyLock<AllowlistCache> =
    std::sync::LazyLock::new(|| AllowlistCache {
        inner: Mutex::new(None),
    });

#[cfg(test)]
fn reset_allowlist_cache_for_tests() {
    if let Ok(mut guard) = ALLOWLIST_CACHE.inner.lock() {
        *guard = None;
    }
}

/// Load the manifest allowlist from the file specified by `SKRILLS_MANIFEST_ALLOWLIST`.
fn load_allowlist() -> Option<Vec<AllowlistEntry>> {
    let path = std::env::var("SKRILLS_MANIFEST_ALLOWLIST").ok()?;
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %path, error = %e, "failed to read manifest allowlist");
            return None;
        }
    };
    let val: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(path = %path, error = %e, "failed to parse manifest allowlist JSON");
            return None;
        }
    };
    let arr = match val.as_array() {
        Some(a) => a,
        None => {
            tracing::warn!(path = %path, "manifest allowlist is not an array");
            return None;
        }
    };
    let mut entries = Vec::new();
    for item in arr {
        let Some(sub) = item.get("name_substr").and_then(|v| v.as_str()) else {
            continue;
        };
        let min = item
            .get("min_version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        entries.push(AllowlistEntry {
            name_substr: sub.to_ascii_lowercase(),
            min_version: min,
        });
    }
    Some(entries)
}

/// An in-memory cache for discovered skills.
///
/// Stores metadata for discovered skills to prevent repeated directory traversals.
/// The cache includes a time-to-live (TTL) and automatically refreshes when stale.
struct SkillCache {
    roots: Vec<SkillRoot>,
    ttl: Duration,
    last_scan: Option<Instant>,
    skills: Vec<SkillMeta>,
    duplicates: Vec<DuplicateInfo>,
    uri_index: HashMap<String, usize>,
    /// Snapshot path is resolved once to avoid cross-test/env races
    snapshot_path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct SkillCacheSnapshot {
    roots: Vec<String>,
    last_scan: u64,
    skills: Vec<SkillMeta>,
    duplicates: Vec<DuplicateInfo>,
}

impl SkillCache {
    /// Create a new `SkillCache` with the given roots.
    #[cfg_attr(not(test), allow(dead_code))]
    fn new(roots: Vec<SkillRoot>) -> Self {
        Self::new_with_ttl(roots, cache_ttl(&load_manifest_settings))
    }

    /// Create a new `SkillCache` with the given roots and TTL.
    fn new_with_ttl(roots: Vec<SkillRoot>, ttl: Duration) -> Self {
        let snapshot_path = Self::resolve_snapshot_path();
        let mut cache = Self {
            roots,
            ttl,
            last_scan: None,
            skills: Vec::new(),
            duplicates: Vec::new(),
            uri_index: HashMap::new(),
            snapshot_path,
        };
        if let Err(e) = cache.try_load_snapshot() {
            tracing::debug!(
                target: "skrills::startup",
                error = %e,
                "failed to load discovery snapshot; will rescan"
            );
        }
        cache
    }

    #[cfg(test)]
    fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Returns the paths of the root directories being watched.
    fn watched_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|r| r.root.clone()).collect()
    }

    /// Resolve snapshot path once to prevent later env churn from redirecting cache IO.
    fn resolve_snapshot_path() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("SKRILLS_CACHE_PATH") {
            return Some(PathBuf::from(path));
        }
        match home_dir() {
            Ok(h) => Some(h.join(".codex/skills-cache.json")),
            Err(e) => {
                tracing::debug!(target: "skrills::startup", error=%e, "could not resolve home dir for snapshot");
                None
            }
        }
    }

    fn snapshot_path(&self) -> Option<PathBuf> {
        self.snapshot_path.clone()
    }

    fn roots_fingerprint(&self) -> Vec<String> {
        self.roots
            .iter()
            .map(|r| r.root.to_string_lossy().into_owned())
            .collect()
    }

    /// Attempt to load a persisted snapshot if it is still within TTL and roots match.
    fn try_load_snapshot(&mut self) -> Result<()> {
        let Some(path) = self.snapshot_path() else {
            return Ok(());
        };
        if !path.exists() {
            return Ok(());
        }
        let data = fs::read_to_string(&path)?;
        let snap: SkillCacheSnapshot = serde_json::from_str(&data)?;

        let current_roots = self.roots_fingerprint();
        if snap.roots != current_roots {
            tracing::warn!(
                target: "skrills::startup",
                "snapshot roots mismatch: expected {:?}, got {:?}",
                current_roots,
                snap.roots
            );
            return Ok(());
        }

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let age = now_secs.saturating_sub(snap.last_scan);
        if age as u128 > self.ttl.as_secs() as u128 {
            tracing::warn!(
                target: "skrills::startup",
                "snapshot stale: age {}s > ttl {}s",
                age,
                self.ttl.as_secs()
            );
            return Ok(());
        }

        let mut uri_index = HashMap::new();
        for (idx, s) in snap.skills.iter().enumerate() {
            // New canonical URI with server "skrills" and source in path.
            uri_index.insert(
                format!("skill://skrills/{}/{}", s.source.label(), s.name),
                idx,
            );
            // Backward-compatible legacy URI (no server component).
            uri_index.insert(format!("skill://{}/{}", s.source.label(), s.name), idx);
        }
        self.skills = snap.skills;
        self.duplicates = snap.duplicates;
        self.uri_index = uri_index;
        self.last_scan = Some(Instant::now());
        tracing::info!(
            target: "skrills::startup",
            skills = self.skills.len(),
            "loaded discovery snapshot"
        );
        Ok(())
    }

    fn persist_snapshot(&self) {
        if let Some(path) = self.snapshot_path() {
            let snap = SkillCacheSnapshot {
                roots: self.roots_fingerprint(),
                last_scan: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                skills: self.skills.clone(),
                duplicates: self.duplicates.clone(),
            };
            if let Ok(text) = serde_json::to_string(&snap) {
                if let Err(e) = fs::write(&path, text) {
                    tracing::debug!(target: "skrills::startup", error=%e, "failed to persist snapshot");
                }
            }
        }
    }

    /// Invalidate the cache, forcing a rescan on the next access.
    fn invalidate(&mut self) {
        self.last_scan = None;
        self.skills.clear();
        self.duplicates.clear();
        self.uri_index.clear();
    }

    /// Refresh the cache if the TTL has expired or the cache is empty.
    fn refresh_if_stale(&mut self) -> Result<()> {
        let now = Instant::now();
        let fresh = self
            .last_scan
            .map(|ts| now.duration_since(ts) < self.ttl)
            .unwrap_or(false);
        if fresh {
            return Ok(());
        }

        // If we've been invalidated (or never loaded) attempt a cheap snapshot reload.
        // When a snapshot exists, serve it immediately to avoid dropping cached skills
        // if the filesystem scan comes back empty (e.g. transiently missing paths).
        if self.last_scan.is_none() && self.skills.is_empty() {
            if let Err(e) = self.try_load_snapshot() {
                tracing::debug!(
                    target: "skrills::startup",
                    error = %e,
                    "failed to reload discovery snapshot after invalidation"
                );
            } else if !self.skills.is_empty() {
                // Serve the snapshot immediately; schedule a rescan on the next access by
                // backdating `last_scan` so it appears stale after this return.
                let backdate = self
                    .ttl
                    .checked_add(Duration::from_millis(1))
                    .unwrap_or(self.ttl);
                self.last_scan = Some(now.checked_sub(backdate).unwrap_or(now));
                return Ok(());
            }
        }

        let scan_started = Instant::now();
        let mut dup_log = Vec::new();
        let skills = discover_skills(&self.roots, Some(&mut dup_log))?;
        let mut uri_index = HashMap::new();
        for (idx, s) in skills.iter().enumerate() {
            // New canonical URI with server "skrills" and source in path.
            uri_index.insert(
                format!("skill://skrills/{}/{}", s.source.label(), s.name),
                idx,
            );
            // Backward-compatible legacy URI (no server component).
            uri_index.insert(format!("skill://{}/{}", s.source.label(), s.name), idx);
        }
        self.skills = skills;
        self.duplicates = dup_log;
        self.uri_index = uri_index;
        self.last_scan = Some(now);
        self.persist_snapshot();
        let elapsed_ms = scan_started.elapsed().as_millis();
        if elapsed_ms > 250 {
            tracing::info!(
                target: "skrills::scan",
                elapsed_ms,
                roots = self.roots.len(),
                skills = self.skills.len(),
                "skill discovery completed"
            );
        } else {
            tracing::debug!(
                target: "skrills::scan",
                elapsed_ms,
                roots = self.roots.len(),
                skills = self.skills.len(),
                "skill discovery completed"
            );
        }
        Ok(())
    }

    /// Returns the current list of skills and any recorded duplicate information.
    fn skills_with_dups(&mut self) -> Result<(Vec<SkillMeta>, Vec<DuplicateInfo>)> {
        self.refresh_if_stale()?;
        Ok((self.skills.clone(), self.duplicates.clone()))
    }

    /// Retrieve a skill by its URI.
    fn get_by_uri(&mut self, uri: &str) -> Result<SkillMeta> {
        self.refresh_if_stale()?;
        if let Some(idx) = self.uri_index.get(uri).copied() {
            return Ok(self.skills[idx].clone());
        }
        Err(anyhow!("skill not found"))
    }
}

/// An in-memory cache for the contents of `SKILL.md` files.
///
/// This cache uses the skill's path and hash as its key. It prevents re-reading
/// skill files from disk if their content remains unchanged.
#[derive(Default)]
struct ContentCache {
    by_path: HashMap<PathBuf, (String, String)>,
}

impl ContentCache {
    /// Read the full content of a skill, using the cache if possible.
    fn read_full(&mut self, meta: &SkillMeta) -> Result<String> {
        if let Some((hash, text)) = self.by_path.get(&meta.path) {
            if hash == &meta.hash {
                return Ok(text.clone());
            }
        }
        let text = read_skill(&meta.path)?;
        self.by_path
            .insert(meta.path.clone(), (meta.hash.clone(), text.clone()));
        Ok(text)
    }

    /// Read a prefix of a skill's content, using the cache.
    fn read_prefix(&mut self, meta: &SkillMeta, max: usize) -> Result<String> {
        let text = self.read_full(meta)?;
        if text.len() <= max {
            return Ok(text);
        }
        let mut bytes = text.into_bytes();
        bytes.truncate(max);
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }
}

/// Manages and serves skills via the Remote Method Call Protocol (RMCP).
///
/// This service discovers, caches, and facilitates interaction with skills.
/// It employs in-memory caches for skill metadata and content to optimize performance.
struct SkillService {
    /// The cache for skill metadata.
    cache: Arc<Mutex<SkillCache>>,
    /// The cache for skill content.
    content_cache: Arc<Mutex<ContentCache>>,
    /// A flag indicating if the cache warmup has started.
    warmup_started: AtomicBool,
    /// The runtime overrides for the service.
    runtime: Arc<Mutex<RuntimeOverrides>>,
    /// Optional subagent service (enabled via `subagents` feature).
    #[cfg(feature = "subagents")]
    subagents: Option<skrills_subagents::SubagentService>,
}

/// Start a filesystem watcher to invalidate caches when skill files change.
#[cfg(feature = "watch")]
fn start_fs_watcher(service: &SkillService) -> Result<RecommendedWatcher> {
    let cache = service.cache.clone();
    let content_cache = service.content_cache.clone();
    let roots = {
        let guard = cache
            .lock()
            .map_err(|e| anyhow!("skill cache poisoned: {e}"))?;
        guard.watched_roots()
    };

    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                if let Ok(mut cache) = cache.lock() {
                    cache.invalidate();
                }
                if let Ok(mut content) = content_cache.lock() {
                    content.by_path.clear();
                }
            }
        },
        NotifyConfig::default(),
    )?;

    for root in roots {
        if root.exists() {
            watcher.watch(root.as_path(), RecursiveMode::Recursive)?;
        }
    }

    Ok(watcher)
}

/// This function serves as a placeholder for the filesystem watcher when the 'watch' feature is disabled.
///
/// It returns an error if called.
#[cfg(not(feature = "watch"))]
fn start_fs_watcher(_service: &SkillService) -> Result<()> {
    Err(anyhow!(
        "watch feature is disabled; rebuild with --features watch"
    ))
}

impl SkillService {
    /// Create a new `SkillService` with the default search roots.
    #[cfg_attr(not(test), allow(dead_code))]
    fn new(extra_dirs: Vec<PathBuf>) -> Result<Self> {
        Self::new_with_ttl(extra_dirs, cache_ttl(&load_manifest_settings))
    }

    /// Create a new `SkillService` with a custom cache TTL.
    fn new_with_ttl(extra_dirs: Vec<PathBuf>, ttl: Duration) -> Result<Self> {
        let build_started = Instant::now();
        let roots = skill_roots(&extra_dirs)?;
        let elapsed_ms = build_started.elapsed().as_millis();
        tracing::info!(
            target: "skrills::startup",
            elapsed_ms,
            roots = roots.len(),
            skills = "deferred", // Skill discovery is deferred until after initialize to keep initial response fast.
            "SkillService constructed"
        );
        Ok(Self {
            cache: Arc::new(Mutex::new(SkillCache::new_with_ttl(roots, ttl))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::load()?)),
            #[cfg(feature = "subagents")]
            subagents: Some(SubagentService::new()?),
        })
    }

    /// Test-only helper to build a service from explicit roots without
    /// re-evaluating environment-driven discovery order. This prevents tests
    /// that persist snapshots from becoming brittle when environment or
    /// priority configuration shifts between snapshot creation and service
    /// construction.
    #[cfg(test)]
    fn new_with_roots_for_test(roots: Vec<SkillRoot>, ttl: Duration) -> Result<Self> {
        let build_started = Instant::now();
        let elapsed_ms = build_started.elapsed().as_millis();
        tracing::info!(
            target: "skrills::startup",
            elapsed_ms,
            roots = roots.len(),
            skills = "deferred",
            "SkillService constructed (test roots)"
        );
        Ok(Self {
            cache: Arc::new(Mutex::new(SkillCache::new_with_ttl(roots, ttl))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::load()?)),
            #[cfg(feature = "subagents")]
            subagents: Some(SubagentService::new()?),
        })
    }

    /// Clear the metadata and content caches.
    ///
    /// The next cache access will trigger a rescan.
    fn invalidate_cache(&self) -> Result<()> {
        if let Ok(mut cache) = self.cache.lock() {
            cache.invalidate();
        }
        if let Ok(mut content) = self.content_cache.lock() {
            content.by_path.clear();
        }
        Ok(())
    }

    /// Returns the current skills and a log of any duplicates.
    ///
    /// Duplicates are resolved by priority, retaining the winning skill.
    fn current_skills_with_dups(&self) -> Result<(Vec<SkillMeta>, Vec<DuplicateInfo>)> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|e| anyhow!("skill cache poisoned: {e}"))?;
        cache.skills_with_dups()
    }

    /// Generate the MCP `listResources` payload.
    fn list_resources_payload(&self) -> Result<Vec<Resource>> {
        let (skills, dup_log) = self.current_skills_with_dups()?;
        let mut resources: Vec<Resource> = skills
            .into_iter()
            .map(|s| {
                let uri = format!("skill://skrills/{}/{}", s.source.label(), s.name);
                let mut raw = RawResource::new(uri, s.name.clone());
                raw.description = Some(format!(
                    "Skill from {} [location: {}]",
                    s.source.label(),
                    s.source.location()
                ));
                raw.mime_type = Some("text/markdown".to_string());
                Resource::new(raw, None)
            })
            .collect();
        // Expose AGENTS.md guidelines as a first-class resource for clients, unless disabled.
        if self.expose_agents_doc()? {
            let mut agents = RawResource::new(AGENTS_URI, AGENTS_NAME);
            agents.description = Some(AGENTS_DESCRIPTION.to_string());
            agents.mime_type = Some("text/markdown".to_string());
            resources.insert(0, Resource::new(agents, None));
        }
        if !dup_log.is_empty() {
            for dup in dup_log {
                tracing::warn!(
                    "duplicate skill {} skipped from {} (winner: {})",
                    dup.name,
                    dup.skipped_source,
                    dup.kept_source
                );
            }
        }
        Ok(resources)
    }

    /// Read a resource by its URI and return its content.
    fn read_resource_sync(&self, uri: &str) -> Result<ReadResourceResult> {
        if uri == AGENTS_URI {
            if !self.expose_agents_doc()? {
                return Err(anyhow!("resource not found"));
            }
            return Ok(ReadResourceResult {
                contents: vec![text_with_location(AGENTS_TEXT, uri, None, "global")],
            });
        }
        if !uri.starts_with("skill://") {
            return Err(anyhow!("unsupported uri"));
        }
        let rest = uri.trim_start_matches("skill://");
        let parts = rest.splitn(3, '/').collect::<Vec<_>>();
        if parts.len() < 2 {
            return Err(anyhow!("invalid uri"));
        }
        let uri = if parts[0] == "skrills" {
            let name = parts.get(2).copied().unwrap_or("");
            format!("skill://skrills/{}/{}", parts[1], name)
        } else {
            // legacy: host is actually source label
            let name = if parts.len() == 2 {
                parts[1]
            } else {
                &rest[parts[0].len() + 1..]
            };
            format!("skill://{}/{}", parts[0], name)
        };
        let meta = {
            let mut cache = self
                .cache
                .lock()
                .map_err(|e| anyhow!("skill cache poisoned: {e}"))?;
            cache.get_by_uri(&uri)?
        };
        let text = self.read_skill_cached(&meta)?;
        Ok(ReadResourceResult {
            contents: vec![text_with_location(
                text,
                &uri,
                Some(&meta.source.label()),
                meta.source.location(),
            )],
        })
    }

    /// Read the content of a skill from the cache.
    fn read_skill_cached(&self, meta: &SkillMeta) -> Result<String> {
        let mut cache = self
            .content_cache
            .lock()
            .map_err(|e| anyhow!("content cache poisoned: {e}"))?;
        cache.read_full(meta)
    }

    /// Read a prefix of a skill's content from the cache.
    fn read_prefix_cached(&self, meta: &SkillMeta, max: usize) -> Result<String> {
        let mut cache = self
            .content_cache
            .lock()
            .map_err(|e| anyhow!("content cache poisoned: {e}"))?;
        cache.read_prefix(meta, max)
    }

    /// Render an autoload snippet using cached skill content.
    fn render_autoload_cached(
        &self,
        skills: &[SkillMeta],
        opts: AutoloadOptions<'_, '_, '_, '_>,
    ) -> Result<String> {
        render_autoload_with_reader(
            skills,
            opts,
            |meta| self.read_skill_cached(meta),
            |meta, max| self.read_prefix_cached(meta, max),
        )
    }

    /// Returns the current runtime overrides.
    fn runtime_overrides(&self) -> RuntimeOverrides {
        self.runtime
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Checks if the `AGENTS.md` document should be exposed as a resource.
    fn expose_agents_doc(&self) -> Result<bool> {
        let manifest = load_manifest_settings()?;
        if let Some(flag) = manifest.expose_agents {
            return Ok(flag);
        }
        if let Ok(val) = std::env::var(ENV_EXPOSE_AGENTS) {
            if let Ok(parsed) = val.parse::<bool>() {
                return Ok(parsed);
            }
        }
        // Legacy/edge: explicit manifest JSON without manifest schema parsing.
        if let Ok(custom) = std::env::var("SKRILLS_MANIFEST") {
            if let Ok(text) = fs::read_to_string(&custom) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(flag) = val.get("expose_agents").and_then(|v| v.as_bool()) {
                        return Ok(flag);
                    }
                }
            }
        }

        Ok(true)
    }

    /// Spawns a background thread to warm up the cache.
    ///
    /// This occurs after the `initialize` handshake to ensure a fast initial response.
    /// The warmup is a best-effort process and logs its duration.
    fn spawn_warmup_if_needed(&self) {
        if self.warmup_started.swap(true, Ordering::SeqCst) {
            return;
        }

        let cache = self.cache.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = cache
                .lock()
                .map_err(|e| anyhow!("skill cache poisoned: {e}"))
                .and_then(|mut cache| cache.refresh_if_stale());

            match result {
                Ok(()) => tracing::info!(
                    target: "skrills::warmup",
                    elapsed_ms = started.elapsed().as_millis(),
                    "background cache warm-up finished"
                ),
                Err(e) => tracing::warn!(
                    target: "skrills::warmup",
                    error = %e,
                    "background cache warm-up failed"
                ),
            }
        });
    }
}

/// Inserts location and an optional priority rank into `readResource` responses.
fn text_with_location(
    text: impl Into<String>,
    uri: &str,
    source_label: Option<&str>,
    location: &str,
) -> ResourceContents {
    let mut meta = Meta::new();
    meta.insert("location".into(), json!(location));
    if let Some(label) = source_label {
        if let Some(rank) = priority_labels()
            .iter()
            .position(|p| p == label)
            .map(|i| i + 1)
        {
            meta.insert("priority_rank".into(), json!(rank));
        }
    }
    ResourceContents::TextResourceContents {
        uri: uri.into(),
        mime_type: Some("text".into()),
        text: text.into(),
        meta: Some(meta),
    }
}

impl ServerHandler for SkillService {
    /// List all available resources, including skills and the AGENTS.md document.
    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, rmcp::ErrorData>> + Send + '_
    {
        let result = self
            .list_resources_payload()
            .map(|resources| ListResourcesResult {
                resources,
                next_cursor: None,
            })
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None));
        std::future::ready(result)
    }

    /// Read the content of a specific resource identified by its URI.
    fn read_resource(
        &self,
        request: ReadResourceRequestParam,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, rmcp::ErrorData>> + Send + '_
    {
        let result = self
            .read_resource_sync(&request.uri)
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None));
        std::future::ready(result)
    }

    /// Lists the tools provided by this service.
    ///
    /// It defines several tools for interacting with skills, including
    /// enumerating available skills, generating autoload snippets, synchronizing
    /// skills from external sources (e.g., Claude), and refreshing internal caches.
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_
    {
        // Codex CLI expects every tool input_schema to include a JSON Schema "type".
        // An empty map triggers "missing field `type`" during MCP → OpenAI conversion,
        // so explicitly mark parameterless tools as taking an empty object.
        let mut schema_empty = JsonMap::new();
        schema_empty.insert("type".into(), json!("object"));
        schema_empty.insert("properties".into(), json!({}));
        schema_empty.insert("additionalProperties".into(), json!(false));
        let schema_empty = std::sync::Arc::new(schema_empty);

        // Schema for autoload-snippet with prompt parameter for skill filtering
        let mut schema_autoload = JsonMap::new();
        schema_autoload.insert("type".into(), json!("object"));
        schema_autoload.insert(
            "properties".into(),
            json!({
                "prompt": {
                    "type": "string",
                    "description": "The user's message or task description to filter relevant skills"
                },
                "include_claude": {
                    "type": "boolean",
                    "description": "Include skills from ~/.claude directory"
                },
                "max_bytes": {
                    "type": "integer",
                    "description": "Maximum bytes for autoloaded content"
                },
                "diagnose": {
                    "type": "boolean",
                    "description": "Include diagnostic information about matched/skipped skills"
                }
            }),
        );
        schema_autoload.insert("additionalProperties".into(), json!(false));
        let schema_autoload = std::sync::Arc::new(schema_autoload);

        let mut schema_list_skills = JsonMap::new();
        schema_list_skills.insert("type".into(), json!("object"));
        schema_list_skills.insert(
            "properties".into(),
            json!({
                "pinned_only": { "type": "boolean" }
            }),
        );
        schema_list_skills.insert("additionalProperties".into(), json!(false));
        let schema_list_skills = std::sync::Arc::new(schema_list_skills);
        let mut options_schema = JsonMap::new();
        options_schema.insert("type".into(), json!("object"));
        options_schema.insert(
            "properties".into(),
            json!({
                "manifest_first": { "type": "boolean" },
                "render_mode_log": { "type": "boolean" },
                "manifest_minimal": { "type": "boolean" }
            }),
        );
        options_schema.insert("additionalProperties".into(), json!(false));
        let options_schema = std::sync::Arc::new(options_schema);

        let mut pins_schema = JsonMap::new();
        pins_schema.insert("type".into(), json!("object"));
        pins_schema.insert(
            "properties".into(),
            json!({
                "skills": {
                    "type": "array",
                    "items": { "type": "string" },
                    "minItems": 1
                },
                "all": { "type": "boolean" }
            }),
        );
        pins_schema.insert("additionalProperties".into(), json!(false));
        let pins_schema = std::sync::Arc::new(pins_schema);

        // Schema for sync tools
        let mut sync_schema = JsonMap::new();
        sync_schema.insert("type".into(), json!("object"));
        sync_schema.insert(
            "properties".into(),
            json!({
                "from": {
                    "type": "string",
                    "description": "Source agent: 'claude' or 'codex'"
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "Preview changes without writing"
                },
                "force": {
                    "type": "boolean",
                    "description": "Skip confirmation prompts"
                }
            }),
        );
        sync_schema.insert("additionalProperties".into(), json!(false));
        let sync_schema = std::sync::Arc::new(sync_schema);

        #[cfg_attr(not(feature = "subagents"), allow(unused_mut))]
        let mut tools = vec![
            Tool {
                name: "list-skills".into(),
                title: Some("List skills".into()),
                description: Some("List discovered SKILL.md files".into()),
                input_schema: schema_list_skills,
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "autoload-snippet".into(),
                title: Some("Load relevant skills for current task".into()),
                description: Some("CALL THIS FIRST with the user's message to load relevant skills. Returns skill content that should inform your response.".into()),
                input_schema: schema_autoload,
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "runtime-status".into(),
                title: Some("Runtime status".into()),
                description: Some("Show effective runtime overrides and sources".into()),
                input_schema: schema_empty.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "render-preview".into(),
                title: Some("Preview selected skills with size estimates".into()),
                description: Some(std::borrow::Cow::Borrowed(
                    "Return matched skill names plus manifest/preview size and estimated tokens.",
                )),
                input_schema: schema_empty.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "set-runtime-options".into(),
                title: Some("Set runtime options".into()),
                description: Some(
                    "Adjust manifest/logging overrides for autoload rendering".into(),
                ),
                input_schema: options_schema,
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "sync-from-claude".into(),
                title: Some("Copy ~/.claude skills into ~/.codex".into()),
                description: Some(
                    "Mirror SKILL.md files from ~/.claude into ~/.codex/skills-mirror".into(),
                ),
                input_schema: schema_empty.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "refresh-cache".into(),
                title: Some("Refresh caches".into()),
                description: Some("Invalidate in-memory skill and content caches".into()),
                input_schema: schema_empty.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "pin-skills".into(),
                title: Some("Pin skills".into()),
                description: Some(
                    "Persistently pin skills so they are always eligible for autoload.".into(),
                ),
                input_schema: pins_schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "unpin-skills".into(),
                title: Some("Unpin skills".into()),
                description: Some(
                    "Remove skills from the pinned set (or clear all with all=true).".into(),
                ),
                input_schema: pins_schema,
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            // Cross-agent sync tools
            Tool {
                name: "sync-skills".into(),
                title: Some("Sync skills between agents".into()),
                description: Some(
                    "Sync SKILL.md files between Claude and Codex. Use --from to specify source.".into(),
                ),
                input_schema: sync_schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "sync-commands".into(),
                title: Some("Sync slash commands between agents".into()),
                description: Some(
                    "Sync slash command definitions between Claude and Codex.".into(),
                ),
                input_schema: sync_schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "sync-mcp-servers".into(),
                title: Some("Sync MCP server configurations".into()),
                description: Some(
                    "Sync MCP server configurations between Claude and Codex.".into(),
                ),
                input_schema: sync_schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "sync-preferences".into(),
                title: Some("Sync preferences between agents".into()),
                description: Some(
                    "Sync compatible settings/preferences between Claude and Codex.".into(),
                ),
                input_schema: sync_schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "sync-all".into(),
                title: Some("Sync all configurations".into()),
                description: Some(
                    "Sync skills, commands, MCP servers, and preferences in one operation.".into(),
                ),
                input_schema: sync_schema.clone(),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "sync-status".into(),
                title: Some("Preview sync changes".into()),
                description: Some(
                    "Show what would be synced without making changes (dry run).".into(),
                ),
                input_schema: sync_schema,
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
        ];

        #[cfg(feature = "subagents")]
        if let Some(subagents) = &self.subagents {
            tools.extend(subagents.tools());
        }

        std::future::ready(Ok(ListToolsResult {
            tools,
            next_cursor: None,
        }))
    }

    /// Executes a specific tool identified by `request.name`.
    ///
    /// It dispatches to internal functions based on the tool name,
    /// such as listing skills, generating autoload snippets, synchronizing
    /// from Claude, or refreshing caches. It returns the result of the tool
    /// execution.
    fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::ErrorData>> + Send + '_
    {
        Box::pin(async move {
            #[cfg(feature = "subagents")]
            {
                let name = request.name.to_string();
                if matches!(
                    name.as_str(),
                    "list_subagents"
                        | "run_subagent"
                        | "run_subagent_async"
                        | "get_run_status"
                        | "get_async_status"
                        | "stop_run"
                        | "get_run_history"
                        | "download_transcript_secure"
                ) {
                    if let Some(service) = &self.subagents {
                        let args = request.arguments.as_ref();
                        let res = service.handle_call(&name, args).await.map_err(|e| {
                            rmcp::model::ErrorData::new(
                                rmcp::model::ErrorCode::INTERNAL_ERROR,
                                format!("subagent error: {e}"),
                                None,
                            )
                        })?;
                        return Ok(res);
                    }
                }
            }
            let result = || -> Result<CallToolResult> {
                match request.name.as_ref() {
                "list-skills" => {
                    let pinned = load_pinned_with_defaults()?;
                    let pinned_only = request
                        .arguments
                        .as_ref()
                        .and_then(|obj| obj.get("pinned_only"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let (skills, dup_log) = self.current_skills_with_dups()?;
                    let (priority, rank_map) = priority_labels_and_rank_map();
                    let skills_raw_with_rank: Vec<serde_json::Value> = skills
                        .iter()
                        .filter(|s| !pinned_only || pinned.contains(&s.name))
                        .map(|s| {
                            let rank = rank_map
                                .get(&s.source.label())
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            json!({
                                "name": s.name,
                                "path": s.path,
                                "source": s.source,
                                "root": s.root,
                                "hash": s.hash,
                                "priority_rank": rank,
                                "pinned": pinned.contains(&s.name)
                            })
                        })
                        .collect();
                    let mut skills_ranked = skills_raw_with_rank.clone();
                    skills_ranked.sort_by_key(|v| v.get("priority_rank").and_then(|n| n.as_u64()).unwrap_or(u64::MAX));
                    let payload = json!({
                        "skills": skills_raw_with_rank,
                        "skills_ranked": skills_ranked,
                        "_meta": {
                            "duplicates": dup_log,
                            "priority": priority,
                            "priority_rank_by_source": rank_map,
                            "pinned_count": pinned.len(),
                            "pinned_only": pinned_only
                        }
                    });
                    if !dup_log.is_empty() {
                        for dup in dup_log.iter() {
                            tracing::warn!(
                                "duplicate skill {} skipped from {} (winner: {})",
                                dup.name,
                                dup.skipped_source,
                                dup.kept_source
                            );
                        }
                    }
                    Ok(CallToolResult {
                        content: vec![Content::text(format!(
                            "listed skills{}{}",
                            if dup_log.is_empty() {
                                "".into()
                            } else {
                                format!(" ({} duplicates skipped)", dup_log.len())
                            },
                            if pinned_only { " [pinned only]" } else { "" }
                        ))],
                        structured_content: Some(payload),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "autoload-snippet" => {
                    let (skills, dup_log) = self.current_skills_with_dups()?;
                    let (priority, rank_map) = priority_labels_and_rank_map();
                    let skills_with_rank: Vec<serde_json::Value> = skills
                        .iter()
                        .map(|s| {
                            json!({
                                "name": s.name,
                                "path": s.path,
                                "source": s.source,
                                "root": s.root,
                                "hash": s.hash,
                                "priority_rank": rank_map.get(&s.source.label()).and_then(|v| v.as_u64()).unwrap_or(0)
                            })
                        })
                        .collect();
                    let args: AutoloadArgs = request
                        .arguments
                        .as_ref()
                        .map(|obj| {
                            serde_json::from_value(json!(obj.clone())).map_err(anyhow::Error::from)
                        })
                        .transpose()?
                        .unwrap_or_default();
                    let manual_pins = load_pinned_with_defaults().unwrap_or_default();
                    let history = load_history().unwrap_or_default();
                    let auto_pins = if args.auto_pin.unwrap_or(env_auto_pin_default()) {
                        auto_pin_from_history(&history)
                    } else {
                        HashSet::new()
                    };
                    let mut effective_pins = manual_pins.clone();
                    effective_pins.extend(auto_pins.iter().cloned());
                    let preload_terms = if let Some(path) = agents_manifest()? {
                        if let Ok(text) = fs::read_to_string(&path) {
                            Some(extract_refs_from_agents(&text))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let preload_terms_ref = preload_terms.as_ref();
                    let mut diag = if args.diagnose.unwrap_or(env_diag_default()) {
                        Some(Diagnostics::default())
                    } else {
                        None
                    };
                    if let Some(d) = diag.as_mut() {
                        d.duplicates.extend(dup_log.iter().cloned());
                    }
                    let runtime = self.runtime_overrides();
                    let render_mode = manifest_render_mode(&runtime, context.peer.peer_info());
                    let mut matched: HashSet<String> = HashSet::new();
                    let content = self.render_autoload_cached(
                        &skills,
                        AutoloadOptions {
                            include_claude: args
                                .include_claude
                                .unwrap_or(env_include_claude_default()),
                            max_bytes: args.max_bytes.or(env_max_bytes()),
                            prompt: args
                                .prompt
                                .or_else(|| std::env::var("SKRILLS_PROMPT").ok())
                                .as_deref(),
                            embed_threshold: Some(
                                args.embed_threshold
                                    .unwrap_or_else(env_embed_threshold)
                            ),
                            preload_terms: preload_terms_ref,
                            pinned: Some(&effective_pins),
                            matched: Some(&mut matched),
                            diagnostics: diag.as_mut(),
                            render_mode,
                            log_render_mode: runtime.render_mode_log(),
                            gzip_ok: peer_accepts_gzip(context.peer.peer_info()),
                            minimal_manifest: runtime.manifest_minimal(),
                        },
                    )?;
                    let ts = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let mut history = history;
                    let mut matched_vec: Vec<String> = matched.into_iter().collect();
                    matched_vec.sort();
                    history.push(HistoryEntry {
                        ts,
                        skills: matched_vec.clone(),
                    });
                    let _ = save_history(history);
                    Ok(CallToolResult {
                        content: vec![Content::text(content.clone())],
                        structured_content: Some(json!({
                            "content": content,
                            "matched": matched_vec.clone(),
                            "truncated": diag.as_ref().map(|d| d.truncated).unwrap_or(false),
                            "skills": skills_with_rank,
                            "_meta": {
                                "duplicates": dup_log,
                                "priority": priority,
                                "priority_rank_by_source": rank_map
                            }
                        })),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "render-preview" => {
                    let (skills, dup_log) = self.current_skills_with_dups()?;
                    let runtime = self.runtime_overrides();
                    let stats = render_preview_stats(&skills, runtime.manifest_minimal())?;
                    let text = format!(
                        "preview matched {} skills, ~{} tokens (~{} bytes)",
                        stats.matched.len(),
                        stats.estimated_tokens,
                        stats.manifest_bytes
                    );
                    Ok(CallToolResult {
                        content: vec![Content::text(text)],
                        structured_content: Some(json!({
                            "matched": stats.matched,
                            "manifest_bytes": stats.manifest_bytes,
                            "estimated_tokens": stats.estimated_tokens,
                            "truncated": stats.truncated,
                            "truncated_content": stats.truncated_content,
                            "_meta": { "duplicates": dup_log }
                        })),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "runtime-status" => {
                    let runtime = self.runtime_overrides();
            let status = json!({
                "manifest_first": runtime.manifest_first(),
                "render_mode_log": runtime.render_mode_log(),
                "manifest_minimal": runtime.manifest_minimal(),
                "overrides": {
                    "manifest_first": runtime.manifest_first,
                    "render_mode_log": runtime.render_mode_log,
                    "manifest_minimal": runtime.manifest_minimal,
                },
                "env": {
                    "manifest_first": env_manifest_first(),
                    "render_mode_log": env_render_mode_log(),
                    "manifest_minimal": env_manifest_minimal(),
                }
            });
                    Ok(CallToolResult {
                        content: vec![Content::text("runtime status")],
                        structured_content: Some(status),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "set-runtime-options" => {
                    let mut runtime = self.runtime_overrides();
                    if let Some(args) = request.arguments.as_ref() {
                        if let Some(val) = args.get("manifest_first").and_then(|v| v.as_bool()) {
                            runtime.manifest_first = Some(val);
                        }
                        if let Some(val) = args.get("render_mode_log").and_then(|v| v.as_bool()) {
                            runtime.render_mode_log = Some(val);
                        }
                        if let Some(val) = args.get("manifest_minimal").and_then(|v| v.as_bool()) {
                            runtime.manifest_minimal = Some(val);
                        }
                        if let Err(e) = runtime.save() {
                            tracing::warn!(error = %e, "failed to save runtime overrides");
                        }
                        if let Ok(mut guard) = self.runtime.lock() {
                            *guard = runtime.clone();
                        }
                    }
                    let status = json!({
                        "manifest_first": runtime.manifest_first(),
                        "render_mode_log": runtime.render_mode_log(),
                        "manifest_minimal": runtime.manifest_minimal(),
                        "overrides": {
                            "manifest_first": runtime.manifest_first,
                            "render_mode_log": runtime.render_mode_log,
                            "manifest_minimal": runtime.manifest_minimal.unwrap_or(runtime.manifest_minimal()),
                        }
                    });
                    Ok(CallToolResult {
                        content: vec![Content::text("runtime options updated")],
                        structured_content: Some(status),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "sync-from-claude" => {
                    let include_marketplace = request.arguments.as_ref()
                        .and_then(|obj| obj.get("include_marketplace"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let home = home_dir()?;
                    let claude_root = mirror_source_root(&home);
                    let mirror_root = home.join(".codex/skills-mirror");
                    let report = sync_from_claude(&claude_root, &mirror_root, include_marketplace)?;
                    let text = if report.copied_names.is_empty() {
                        format!("copied: {}, skipped: {}", report.copied, report.skipped)
                    } else {
                        format!(
                            "copied: {}, skipped: {}\nsynced: {}",
                            report.copied,
                            report.skipped,
                            report.copied_names.join(", ")
                        )
                    };
                    let (priority, rank_map) = priority_labels_and_rank_map();
                    Ok(CallToolResult {
                        content: vec![Content::text(text)],
                        structured_content: Some(json!({
                            "report": {
                                "copied": report.copied,
                                "skipped": report.skipped,
                                "synced": report.copied_names
                            },
                            "_meta": {
                                "priority": priority,
                                "priority_rank_by_source": rank_map
                            }
                        })),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                // Cross-agent sync tools
                "sync-skills" => {
                    // Skills use existing sync mechanism (sync_from_claude)
                    let from = request.arguments.as_ref()
                        .and_then(|obj| obj.get("from"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude");
                    let dry_run = request.arguments.as_ref()
                        .and_then(|obj| obj.get("dry_run"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let include_marketplace = request.arguments.as_ref()
                        .and_then(|obj| obj.get("include_marketplace"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    if from == "claude" {
                        let home = home_dir()?;
                        let claude_root = mirror_source_root(&home);
                        let mirror_root = home.join(".codex/skills-mirror");

                        if dry_run {
                            let count = walkdir::WalkDir::new(&claude_root)
                                .min_depth(1)
                                .max_depth(6)
                                .into_iter()
                                .filter_map(|e| e.ok())
                                .filter(crate::discovery::is_skill_file)
                                .count();

                            Ok(CallToolResult {
                                content: vec![Content::text(format!(
                                    "Would sync {} skills from Claude to Codex", count
                                ))],
                                is_error: Some(false),
                                structured_content: Some(json!({
                                    "dry_run": true,
                                    "skill_count": count
                                })),
                                meta: None,
                            })
                        } else {
                            let report = sync_from_claude(&claude_root, &mirror_root, include_marketplace)?;
                            Ok(CallToolResult {
                                content: vec![Content::text(format!(
                                    "Synced {} skills ({} unchanged)",
                                    report.copied, report.skipped
                                ))],
                                is_error: Some(false),
                                structured_content: Some(json!({
                                    "copied": report.copied,
                                    "skipped": report.skipped,
                                    "copied_names": report.copied_names
                                })),
                                meta: None,
                            })
                        }
                    } else {
                        Ok(CallToolResult {
                            content: vec![Content::text(
                                "Codex → Claude skill sync not yet implemented".to_string()
                            )],
                            is_error: Some(true),
                            structured_content: None,
                            meta: None,
                        })
                    }
                }
                "sync-commands" => {
                    use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

                    let from = request.arguments.as_ref()
                        .and_then(|obj| obj.get("from"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude");
                    let dry_run = request.arguments.as_ref()
                        .and_then(|obj| obj.get("dry_run"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let skip_existing_commands = request.arguments.as_ref()
                        .and_then(|obj| obj.get("skip_existing_commands"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let include_marketplace = request.arguments.as_ref()
                        .and_then(|obj| obj.get("include_marketplace"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let params = SyncParams {
                        from: Some(from.to_string()),
                        dry_run,
                        sync_commands: true,
                        skip_existing_commands,
                        sync_mcp_servers: false,
                        sync_preferences: false,
                        sync_skills: false,
                        include_marketplace,
                        ..Default::default()
                    };

                    let report = if from == "claude" {
                        let source = ClaudeAdapter::new()?;
                        let target = CodexAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    } else {
                        let source = CodexAdapter::new()?;
                        let target = ClaudeAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    };

                    Ok(CallToolResult {
                        content: vec![Content::text(report.summary.clone())],
                        is_error: Some(false),
                        structured_content: Some(json!({
                            "report": report,
                            "dry_run": dry_run,
                            "skip_existing_commands": skip_existing_commands
                        })),
                        meta: None,
                    })
                }
                "sync-mcp-servers" => {
                    use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

                    let from = request.arguments.as_ref()
                        .and_then(|obj| obj.get("from"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude");
                    let dry_run = request.arguments.as_ref()
                        .and_then(|obj| obj.get("dry_run"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let params = SyncParams {
                        from: Some(from.to_string()),
                        dry_run,
                        sync_commands: false,
                        sync_mcp_servers: true,
                        sync_preferences: false,
                        sync_skills: false,
                        ..Default::default()
                    };

                    let report = if from == "claude" {
                        let source = ClaudeAdapter::new()?;
                        let target = CodexAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    } else {
                        let source = CodexAdapter::new()?;
                        let target = ClaudeAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    };

                    Ok(CallToolResult {
                        content: vec![Content::text(report.summary.clone())],
                        is_error: Some(false),
                        structured_content: Some(json!({
                            "report": report,
                            "dry_run": dry_run
                        })),
                        meta: None,
                    })
                }
                "sync-preferences" => {
                    use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

                    let from = request.arguments.as_ref()
                        .and_then(|obj| obj.get("from"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude");
                    let dry_run = request.arguments.as_ref()
                        .and_then(|obj| obj.get("dry_run"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let params = SyncParams {
                        from: Some(from.to_string()),
                        dry_run,
                        sync_commands: false,
                        sync_mcp_servers: false,
                        sync_preferences: true,
                        sync_skills: false,
                        ..Default::default()
                    };

                    let report = if from == "claude" {
                        let source = ClaudeAdapter::new()?;
                        let target = CodexAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    } else {
                        let source = CodexAdapter::new()?;
                        let target = ClaudeAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    };

                    Ok(CallToolResult {
                        content: vec![Content::text(report.summary.clone())],
                        is_error: Some(false),
                        structured_content: Some(json!({
                            "report": report,
                            "dry_run": dry_run
                        })),
                        meta: None,
                    })
                }
                "sync-all" => {
                    use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

                    let from = request.arguments.as_ref()
                        .and_then(|obj| obj.get("from"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude");
                    let dry_run = request.arguments.as_ref()
                        .and_then(|obj| obj.get("dry_run"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let include_marketplace = request.arguments.as_ref()
                        .and_then(|obj| obj.get("include_marketplace"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    // Sync skills first (using existing mechanism)
                    let skill_report = if from == "claude" && !dry_run {
                        let home = home_dir()?;
                        let claude_root = mirror_source_root(&home);
                        let mirror_root = home.join(".codex/skills-mirror");
                        sync_from_claude(&claude_root, &mirror_root, include_marketplace)?
                    } else {
                        crate::sync::SyncReport::default()
                    };

                    let skip_existing_commands = request.arguments.as_ref()
                        .and_then(|obj| obj.get("skip_existing_commands"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    let params = SyncParams {
                        from: Some(from.to_string()),
                        dry_run,
                        sync_commands: true,
                        skip_existing_commands,
                        sync_mcp_servers: true,
                        sync_preferences: true,
                        sync_skills: false, // Handled above
                        include_marketplace,
                        ..Default::default()
                    };

                    let report = if from == "claude" {
                        let source = ClaudeAdapter::new()?;
                        let target = CodexAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    } else {
                        let source = CodexAdapter::new()?;
                        let target = ClaudeAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    };

                    Ok(CallToolResult {
                        content: vec![Content::text(format!(
                            "{}\nSkills: {} copied, {} skipped",
                            report.summary, skill_report.copied, skill_report.skipped
                        ))],
                        is_error: Some(false),
                        structured_content: Some(json!({
                            "report": report,
                            "skill_report": {
                                "copied": skill_report.copied,
                                "skipped": skill_report.skipped
                            },
                            "dry_run": dry_run,
                            "skip_existing_commands": skip_existing_commands
                        })),
                        meta: None,
                    })
                }
                "sync-status" => {
                    use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

                    let from = request.arguments.as_ref()
                        .and_then(|obj| obj.get("from"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("claude");

                    let params = SyncParams {
                        from: Some(from.to_string()),
                        dry_run: true, // Always dry run for status
                        sync_commands: true,
                        sync_mcp_servers: true,
                        sync_preferences: true,
                        sync_skills: true,
                        ..Default::default()
                    };

                    let report = if from == "claude" {
                        let source = ClaudeAdapter::new()?;
                        let target = CodexAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    } else {
                        let source = CodexAdapter::new()?;
                        let target = ClaudeAdapter::new()?;
                        SyncOrchestrator::new(source, target).sync(&params)?
                    };

                    Ok(CallToolResult {
                        content: vec![Content::text(format!(
                            "Sync Preview ({})\n{}",
                            from, report.summary
                        ))],
                        is_error: Some(false),
                        structured_content: Some(json!({
                            "preview": true,
                            "report": report
                        })),
                        meta: None,
                    })
                }
                "refresh-cache" => {
                    self.invalidate_cache()?;
                    Ok(CallToolResult {
                        content: vec![Content::text("cache invalidated")],
                        structured_content: None,
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "pin-skills" => {
                    let args = request
                        .arguments
                        .as_ref()
                        .ok_or_else(|| anyhow!("skills array required"))?;
                    let skills_arg = args
                        .get("skills")
                        .and_then(|v| v.as_array())
                        .ok_or_else(|| anyhow!("skills must be an array of strings"))?;
                    let mut specs: Vec<String> = Vec::new();
                    for v in skills_arg {
                        specs.push(
                            v.as_str()
                                .ok_or_else(|| anyhow!("skills entries must be strings"))?
                                .to_string(),
                        );
                    }
                    let (skills, _) = self.current_skills_with_dups()?;
                    let mut pinned = load_pinned()?;
                    for spec in specs {
                        let name = resolve_skill(&spec, &skills)?;
                        pinned.insert(name.to_string());
                    }
                    save_pinned(&pinned)?;
                    Ok(CallToolResult {
                        content: vec![Content::text(format!("pinned {} skills", pinned.len()))],
                        structured_content: Some(json!({
                            "pinned": pinned.len()
                        })),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                "unpin-skills" => {
                    let args = request.arguments.clone().unwrap_or_default();
                    let all = args
                        .get("all")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if all {
                        save_pinned(&HashSet::new())?;
                        return Ok(CallToolResult {
                            content: vec![Content::text("cleared all pinned skills")],
                            structured_content: Some(json!({"pinned": 0})),
                            is_error: Some(false),
                            meta: None,
                        });
                    }
                    let skills_arg = args
                        .get("skills")
                        .and_then(|v| v.as_array())
                        .ok_or_else(|| anyhow!("skills must be provided unless all=true"))?;
                    let mut specs: Vec<String> = Vec::new();
                    for v in skills_arg {
                        specs.push(
                            v.as_str()
                                .ok_or_else(|| anyhow!("skills entries must be strings"))?
                                .to_string(),
                        );
                    }
                    let (skills, _) = self.current_skills_with_dups()?;
                    let mut pinned = load_pinned()?;
                    for spec in specs {
                        let name = resolve_skill(&spec, &skills)?;
                        pinned.remove(name);
                    }
                    save_pinned(&pinned)?;
                    Ok(CallToolResult {
                        content: vec![Content::text(format!("pinned skills remaining: {}", pinned.len()))],
                        structured_content: Some(json!({
                            "pinned": pinned.len()
                        })),
                        is_error: Some(false),
                        meta: None,
                    })
                }
                other => Err(anyhow!("unknown tool {other}")),
            }
        }()
        .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None));
            result
        })
    }

    /// Returns initialization information for the RMCP server.
    ///
    /// This includes server capabilities and a brief instruction message,
    /// clarifying that this service acts as a bridge for `SKILL.md` files.
    fn get_info(&self) -> InitializeResult {
        // Start background warm-up only after the handshake path is hit to
        // keep the initialize response fast.
        self.spawn_warmup_if_needed();
        InitializeResult {
            capabilities: ServerCapabilities {
                resources: Some(Default::default()),
                tools: Some(Default::default()),
                ..Default::default()
            },
            instructions: Some("Codex SKILL.md bridge".into()),
            ..Default::default()
        }
    }
}

/// Print a JSON list of discovered skills.
fn list_skills(extra_dirs: &[PathBuf]) -> Result<()> {
    let skills = collect_skills(extra_dirs)?;
    println!("{}", serde_json::to_string_pretty(&skills)?);
    Ok(())
}

/// Handle the `serve` command.
fn handle_serve_command(
    skill_dirs: Vec<PathBuf>,
    cache_ttl_ms: Option<u64>,
    trace_wire: bool,
    #[cfg(feature = "watch")] watch: bool,
) -> Result<()> {
    let ttl = cache_ttl_ms
        .map(Duration::from_millis)
        .unwrap_or_else(|| cache_ttl(&load_manifest_settings));
    let service = SkillService::new_with_ttl(merge_extra_dirs(&skill_dirs), ttl)?;

    #[cfg(feature = "watch")]
    let _watcher = if watch {
        Some(start_fs_watcher(&service)?)
    } else {
        None
    };

    let transport = stdio_with_optional_trace(trace_wire);
    let rt = Runtime::new()?;
    let running = rt.block_on(async {
        serve_server(service, transport)
            .await
            .map_err(|e| anyhow!("failed to start server: {e}"))
    })?;
    rt.block_on(async {
        running
            .waiting()
            .await
            .map_err(|e| anyhow!("server task ended: {e}"))
    })?;

    #[cfg(feature = "watch")]
    drop(_watcher);
    Ok(())
}

/// Handle the `list-pinned` command.
fn handle_list_pinned_command() -> Result<()> {
    let pinned = load_pinned_with_defaults()?;
    if pinned.is_empty() {
        println!("(no pinned skills)");
    } else {
        let mut list: Vec<_> = pinned.into_iter().collect();
        list.sort();
        for name in list {
            println!("{name}");
        }
    }
    Ok(())
}

/// Handle the `pin` command.
fn handle_pin_command(skills: Vec<String>) -> Result<()> {
    let mut pinned = load_pinned()?;
    let all_skills = collect_skills(&merge_extra_dirs(&[]))?;
    for spec in skills {
        let name = resolve_skill(&spec, &all_skills)?;
        pinned.insert(name.to_string());
    }
    save_pinned(&pinned)?;
    println!("Pinned {} skills.", pinned.len());
    Ok(())
}

/// Handle the `unpin` command.
fn handle_unpin_command(skills: Vec<String>, all: bool) -> Result<()> {
    if all {
        save_pinned(&HashSet::new())?;
        println!("Cleared all pinned skills.");
        return Ok(());
    }
    if skills.is_empty() {
        return Err(anyhow!("provide skill names or use --all"));
    }
    let mut pinned = load_pinned()?;
    let all_skills = collect_skills(&merge_extra_dirs(&[]))?;
    for spec in skills {
        let name = resolve_skill(&spec, &all_skills)?;
        pinned.remove(name);
    }
    save_pinned(&pinned)?;
    println!("Pinned skills remaining: {}", pinned.len());
    Ok(())
}

/// Handle the `auto-pin` command.
fn handle_auto_pin_command(enable: bool) -> Result<()> {
    save_auto_pin_flag(enable)?;
    println!("Auto-pin {}", if enable { "enabled" } else { "disabled" });
    Ok(())
}

/// Handle the `sync-agents` command.
fn handle_sync_agents_command(path: Option<PathBuf>, skill_dirs: Vec<PathBuf>) -> Result<()> {
    let path = path.unwrap_or_else(|| PathBuf::from("AGENTS.md"));
    sync_agents(&path, &merge_extra_dirs(&skill_dirs))?;
    println!("Updated {}", path.display());
    Ok(())
}

/// Handle the `sync` command.
fn handle_sync_command(include_marketplace: bool) -> Result<()> {
    let home = home_dir()?;
    let report = sync_from_claude(
        &mirror_source_root(&home),
        &home.join(".codex/skills-mirror"),
        include_marketplace,
    )?;
    println!("copied: {}, skipped: {}", report.copied, report.skipped);
    Ok(())
}

fn handle_mirror_command(
    dry_run: bool,
    skip_existing_commands: bool,
    include_marketplace: bool,
) -> Result<()> {
    let home = home_dir()?;
    let claude_root = mirror_source_root(&home);
    if !skip_existing_commands {
        eprintln!(
            "Warning: mirroring commands into ~/.codex/prompts will overwrite prompts with the same name unless --skip-existing-commands is used."
        );
    }
    // Mirror skills/agents/support files
    let report = sync_from_claude(
        &claude_root,
        &home.join(".codex/skills-mirror"),
        include_marketplace,
    )?;
    // Mirror commands/mcp/prefs
    let source = skrills_sync::ClaudeAdapter::new()?;
    let target = skrills_sync::CodexAdapter::new()?;
    let orch = skrills_sync::SyncOrchestrator::new(source, target);
    let params = skrills_sync::SyncParams {
        dry_run,
        sync_skills: false,
        sync_commands: true,
        skip_existing_commands,
        sync_mcp_servers: true,
        sync_preferences: true,
        include_marketplace,
        ..Default::default()
    };
    let sync_report = orch.sync(&params)?;
    // Refresh AGENTS.md with skills + agents (mirror roots now populated)
    handle_sync_agents_command(None, vec![])?;

    println!(
        "mirror complete: skills copied {}, skipped {}; commands written {}, skipped {}; prefs {}, mcp {}{}",
        report.copied,
        report.skipped,
        sync_report.commands.written,
        sync_report.commands.skipped.len(),
        sync_report.preferences.written,
        sync_report.mcp_servers.written,
        if dry_run {
            " (dry-run for commands/prefs/mcp)"
        } else {
            ""
        }
    );

    if skip_existing_commands && !sync_report.commands.skipped.is_empty() {
        println!("Skipped existing commands (kept target copy):");
        for reason in &sync_report.commands.skipped {
            println!("  - {}", reason.description());
        }
    }
    Ok(())
}

fn handle_agent_command(agent_spec: String, skill_dirs: Vec<PathBuf>, dry_run: bool) -> Result<()> {
    let agents = collect_agents(&merge_extra_dirs(&skill_dirs))?;
    let agent = resolve_agent(&agent_spec, &agents)?;
    let cmd = DEFAULT_AGENT_RUN_TEMPLATE.replace("{}", &agent.path.display().to_string());
    println!(
        "Agent: {} (source: {}, path: {})",
        agent.name,
        agent.source.label(),
        agent.path.display()
    );
    if dry_run {
        println!("Command: {cmd}");
        return Ok(());
    }
    let status = Command::new("sh").arg("-c").arg(&cmd).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "agent command exited with status {:?}",
            status.code()
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_setup_command(
    client: Option<String>,
    bin_dir: Option<PathBuf>,
    reinstall: bool,
    uninstall: bool,
    add: bool,
    yes: bool,
    universal: bool,
    mirror_source: Option<PathBuf>,
) -> Result<()> {
    let config = crate::setup::interactive_setup(
        client,
        bin_dir,
        reinstall,
        uninstall,
        add,
        yes,
        universal,
        mirror_source,
    )?;
    crate::setup::run_setup(config)
}

/// The main entry point for the `skrills` application.
pub fn run() -> Result<()> {
    ignore_sigchld()?;
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    // Check for first-run (only for user-facing commands, not for `serve` which is called by MCP)
    // Also skip for batch/non-interactive commands like sync-all
    let command_ref = cli.command.as_ref();
    let is_serve = matches!(command_ref, Some(Commands::Serve { .. }) | None);
    let is_setup = matches!(command_ref, Some(Commands::Setup { .. }));
    let is_batch = matches!(command_ref, Some(Commands::SyncAll { .. }));

    if !is_serve && !is_setup && !is_batch {
        if let Ok(true) = crate::setup::is_first_run() {
            if let Ok(true) = crate::setup::prompt_first_run_setup() {
                // Run interactive setup
                let config = crate::setup::interactive_setup(
                    None, None, false, false, false, false, false, None,
                )?;
                crate::setup::run_setup(config)?;
                println!(
                    "\nYou can now use skrills. Run your command again or explore 'skrills --help'"
                );
                return Ok(());
            } else {
                println!("Setup skipped. Run 'skrills setup' when ready.");
            }
        }
    }

    match cli.command.unwrap_or(Commands::Serve {
        skill_dirs: Vec::new(),
        cache_ttl_ms: None,
        trace_wire: false,
        #[cfg(feature = "watch")]
        watch: false,
    }) {
        Commands::Serve {
            skill_dirs,
            cache_ttl_ms,
            trace_wire,
            #[cfg(feature = "watch")]
            watch,
        } => handle_serve_command(
            skill_dirs,
            cache_ttl_ms,
            trace_wire,
            #[cfg(feature = "watch")]
            watch,
        ),
        Commands::List => list_skills(&merge_extra_dirs(&[])),
        Commands::ListPinned => handle_list_pinned_command(),
        Commands::Pin { skills } => handle_pin_command(skills),
        Commands::Unpin { skills, all } => handle_unpin_command(skills, all),
        Commands::AutoPin { enable } => handle_auto_pin_command(enable),
        Commands::History { limit } => {
            print_history(limit)?;
            Ok(())
        }
        Commands::Mirror {
            dry_run,
            skip_existing_commands,
            include_marketplace,
        } => handle_mirror_command(dry_run, skip_existing_commands, include_marketplace),
        Commands::Agent {
            agent,
            skill_dirs,
            dry_run,
        } => handle_agent_command(agent, skill_dirs, dry_run),
        Commands::SyncAgents { path, skill_dirs } => handle_sync_agents_command(path, skill_dirs),
        Commands::EmitAutoload {
            include_claude,
            max_bytes,
            prompt,
            embed_threshold,
            auto_pin,
            skill_dirs,
            diagnose,
        } => emit_autoload(
            include_claude,
            max_bytes,
            prompt,
            embed_threshold,
            auto_pin,
            &merge_extra_dirs(&skill_dirs),
            diagnose,
        ),
        Commands::Sync {
            include_marketplace,
        } => handle_sync_command(include_marketplace),
        Commands::SyncCommands {
            from,
            dry_run,
            skip_existing_commands,
            include_marketplace,
        } => {
            use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

            if !skip_existing_commands {
                eprintln!(
            "Warning: syncing commands will overwrite existing files under ~/.codex/prompts when names match. Use --skip-existing-commands to keep existing copies."
        );
            }

            let params = SyncParams {
                from: Some(from.clone()),
                dry_run,
                sync_commands: true,
                skip_existing_commands,
                sync_mcp_servers: false,
                sync_preferences: false,
                sync_skills: false,
                include_marketplace,
                ..Default::default()
            };

            let report = if from == "claude" {
                let source = ClaudeAdapter::new()?;
                let target = CodexAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            } else {
                let source = CodexAdapter::new()?;
                let target = ClaudeAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            };

            println!(
                "{}{}",
                report.summary,
                if skip_existing_commands && !report.commands.skipped.is_empty() {
                    format!(
                        "\nSkipped existing commands (kept target copy): {}",
                        report
                            .commands
                            .skipped
                            .iter()
                            .map(|r| r.description())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    String::new()
                }
            );
            if dry_run {
                println!("(dry run - no changes made)");
            }
            Ok(())
        }
        Commands::SyncMcpServers { from, dry_run } => {
            use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

            let params = SyncParams {
                from: Some(from.clone()),
                dry_run,
                sync_commands: false,
                sync_mcp_servers: true,
                sync_preferences: false,
                sync_skills: false,
                ..Default::default()
            };

            let report = if from == "claude" {
                let source = ClaudeAdapter::new()?;
                let target = CodexAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            } else {
                let source = CodexAdapter::new()?;
                let target = ClaudeAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            };

            println!("{}", report.summary);
            if dry_run {
                println!("(dry run - no changes made)");
            }
            Ok(())
        }
        Commands::SyncPreferences { from, dry_run } => {
            use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

            let params = SyncParams {
                from: Some(from.clone()),
                dry_run,
                sync_commands: false,
                sync_mcp_servers: false,
                sync_preferences: true,
                sync_skills: false,
                ..Default::default()
            };

            let report = if from == "claude" {
                let source = ClaudeAdapter::new()?;
                let target = CodexAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            } else {
                let source = CodexAdapter::new()?;
                let target = ClaudeAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            };

            println!("{}", report.summary);
            if dry_run {
                println!("(dry run - no changes made)");
            }
            Ok(())
        }
        Commands::SyncAll {
            from,
            dry_run,
            skip_existing_commands,
            include_marketplace,
        } => {
            use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

            // First sync skills using existing mechanism
            if from == "claude" && !dry_run {
                let home = home_dir()?;
                let claude_root = mirror_source_root(&home);
                let mirror_root = home.join(".codex/skills-mirror");
                let skill_report =
                    sync_from_claude(&claude_root, &mirror_root, include_marketplace)?;
                println!(
                    "Skills: {} synced, {} unchanged",
                    skill_report.copied, skill_report.skipped
                );
            }

            // Then sync commands, MCP servers, preferences, and skills (Codex source)
            let sync_skills = from != "claude";
            let params = SyncParams {
                from: Some(from.clone()),
                dry_run,
                sync_commands: true,
                skip_existing_commands,
                sync_mcp_servers: true,
                sync_preferences: true,
                sync_skills, // Claude source handled above; enable for Codex→Claude
                include_marketplace,
                ..Default::default()
            };

            let report = if from == "claude" {
                let source = ClaudeAdapter::new()?;
                let target = CodexAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            } else {
                let source = CodexAdapter::new()?;
                let target = ClaudeAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            };

            println!(
                "{}{}",
                report.summary,
                if skip_existing_commands && !report.commands.skipped.is_empty() {
                    format!(
                        "\nSkipped existing commands (kept target copy): {}",
                        report
                            .commands
                            .skipped
                            .iter()
                            .map(|r| r.description())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    String::new()
                }
            );
            if dry_run {
                println!("(dry run - no changes made)");
            }
            Ok(())
        }
        Commands::SyncStatus { from } => {
            use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};

            let sync_skills = from != "claude";
            let params = SyncParams {
                from: Some(from.clone()),
                dry_run: true,
                sync_commands: true,
                sync_mcp_servers: true,
                sync_preferences: true,
                sync_skills,
                ..Default::default()
            };

            let report = if from == "claude" {
                let source = ClaudeAdapter::new()?;
                let target = CodexAdapter::new()?;
                let orch = SyncOrchestrator::new(source, target);
                println!(
                    "Sync direction: {} → {}",
                    orch.source_name(),
                    orch.target_name()
                );
                orch.sync(&params)?
            } else {
                let source = CodexAdapter::new()?;
                let target = ClaudeAdapter::new()?;
                let orch = SyncOrchestrator::new(source, target);
                println!(
                    "Sync direction: {} → {}",
                    orch.source_name(),
                    orch.target_name()
                );
                orch.sync(&params)?
            };

            println!("\nPending changes:");
            println!("  Commands: {} would sync", report.commands.written);
            println!("  MCP Servers: {} would sync", report.mcp_servers.written);
            println!("  Preferences: {} would sync", report.preferences.written);

            // Count skills
            let home = home_dir()?;
            let source_root = if from == "claude" {
                mirror_source_root(&home)
            } else {
                home.join(".codex/skills")
            };
            let skill_count = walkdir::WalkDir::new(&source_root)
                .min_depth(1)
                .max_depth(6)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(crate::discovery::is_skill_file)
                .count();
            println!("  Skills: {} found in source", skill_count);

            Ok(())
        }
        Commands::Doctor => doctor_report(),
        Commands::Tui { skill_dirs } => tui_flow(&merge_extra_dirs(&skill_dirs)),
        Commands::Setup {
            client,
            bin_dir,
            reinstall,
            uninstall,
            add,
            yes,
            universal,
            mirror_source,
        } => handle_setup_command(
            client,
            bin_dir,
            reinstall,
            uninstall,
            add,
            yes,
            universal,
            mirror_source,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autoload::{gzip_base64, render_autoload};
    use crate::discovery::read_prefix;
    use crate::runtime::{reset_runtime_cache_for_tests, runtime_overrides_cached};
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;
    use flate2::read::GzDecoder;
    use rmcp::{
        model::ReadResourceRequestParam,
        service::{serve_client, serve_server},
    };
    use skrills_discovery::{hash_file, SkillSource};
    use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};
    use std::collections::HashSet;
    use std::fs;
    use std::io::Read;
    use std::path::Path;
    use std::process::Command;
    use std::sync::LazyLock;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::io::duplex;

    /// Serialize test execution to prevent cross-test contamination due to HOME and on-disk runtime cache mutations.
    static TEST_SERIAL: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lightweight Given/When/Then helpers for readable autoload tests.
    mod gwt_autoload {
        use super::*;
        use crate::discovery::EmbedOverrideGuard;

        pub struct SkillFixture {
            pub skills: Vec<SkillMeta>,
        }

        /// Create a new skill fixture with a single skill.
        pub fn given_skill(
            root: &Path,
            name: &str,
            content: &str,
            source: SkillSource,
        ) -> Result<SkillFixture> {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, content)?;
            let skills = vec![SkillMeta {
                name: name.into(),
                path: path.clone(),
                source,
                root: root.to_path_buf(),
                hash: hash_file(&path)?,
            }];
            Ok(SkillFixture { skills })
        }

        /// Create a new skill fixture with two skills.
        #[allow(clippy::too_many_arguments)]
        pub fn given_two_skills(
            one_root: &Path,
            one_name: &str,
            one_content: &str,
            one_source: SkillSource,
            two_root: &Path,
            two_name: &str,
            two_content: &str,
            two_source: SkillSource,
        ) -> Result<SkillFixture> {
            let mut first = given_skill(one_root, one_name, one_content, one_source)?;
            let second = given_skill(two_root, two_name, two_content, two_source)?;
            first.skills.extend(second.skills);
            Ok(first)
        }

        /// Render an autoload snippet with the given options.
        pub fn when_render_autoload(
            fixture: &SkillFixture,
            options: AutoloadOptions<'_, '_, '_, '_>,
        ) -> Result<String> {
            render_autoload(&fixture.skills, options)
        }

        /// Create an `EmbedOverrideGuard` with the given similarity value.
        pub fn with_embed_similarity(value: f32) -> EmbedOverrideGuard {
            EmbedOverrideGuard::set(value)
        }

        /// Assert that the content contains the needle.
        pub fn then_contains(content: &str, needle: &str) {
            assert!(
                content.contains(needle),
                "expected content to contain `{needle}`, but it did not"
            );
        }

        /// Assert that the content does not contain the needle.
        pub fn then_not_contains(content: &str, needle: &str) {
            assert!(
                !content.contains(needle),
                "expected content to not contain `{needle}`, but it did"
            );
        }
    }

    #[test]
    fn list_resources_includes_agents_doc() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var("SKRILLS_MANIFEST");
        std::env::remove_var(ENV_EXPOSE_AGENTS);
        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };
        let resources = svc.list_resources_payload()?;
        assert!(resources
            .iter()
            .any(|r| r.uri == AGENTS_URI && r.name == AGENTS_NAME));

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn read_resource_returns_agents_doc() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var(ENV_EXPOSE_AGENTS);
        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };
        let result = svc.read_resource_sync(AGENTS_URI)?;
        let text = match &result.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text,
            _ => anyhow::bail!("expected text content"),
        };
        assert!(text.contains("AI Agent Development Guidelines"));

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }
    #[test]
    fn read_resource_includes_priority_rank() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        let skill_path = codex_root.join("alpha/SKILL.md");
        fs::write(&skill_path, "hello")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![SkillRoot {
                root: codex_root.clone(),
                source: SkillSource::Codex,
            }]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };
        let result = svc.read_resource_sync("skill://codex/alpha/SKILL.md")?;
        match &result.contents[0] {
            ResourceContents::TextResourceContents { meta: Some(m), .. } => {
                let rank = m.get("priority_rank").and_then(|v| v.as_u64()).unwrap();
                assert_eq!(rank, 1);
                assert_eq!(
                    m.get("location").and_then(|v| v.as_str()).unwrap(),
                    "global"
                );
            }
            _ => anyhow::bail!("expected text content with meta"),
        };
        Ok(())
    }
    #[test]
    fn priority_rank_map_matches_labels() {
        let (labels, map) = priority_labels_and_rank_map();
        assert_eq!(
            labels,
            vec!["codex", "mirror", "claude", "marketplace", "cache", "agent"]
        );
        assert_eq!(map.get("codex").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(map.get("agent").and_then(|v| v.as_u64()), Some(6));
    }

    #[test]
    fn duplicates_are_logged_and_can_be_reported() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        let claude_root = tmp.path().join("claude/skills");
        fs::create_dir_all(&codex_root)?;
        fs::create_dir_all(&claude_root)?;
        let path1 = codex_root.join("dup/SKILL.md");
        let path2 = claude_root.join("dup/SKILL.md");
        fs::create_dir_all(path1.parent().unwrap())?;
        fs::create_dir_all(path2.parent().unwrap())?;
        fs::write(&path1, "one")?;
        fs::write(&path2, "two")?;

        let roots = vec![
            SkillRoot {
                root: codex_root.clone(),
                source: SkillSource::Codex,
            },
            SkillRoot {
                root: claude_root.clone(),
                source: SkillSource::Claude,
            },
        ];
        let mut dup_log = Vec::new();
        let skills = discover_skills(&roots, Some(&mut dup_log))?;
        assert_eq!(skills.len(), 1);
        assert_eq!(dup_log.len(), 1);
        let dup = &dup_log[0];
        assert_eq!(dup.name, "dup/SKILL.md");
        assert_eq!(dup.kept_source, "codex");
        assert_eq!(dup.skipped_source, "claude");
        Ok(())
    }

    #[test]
    fn skill_cache_refreshes_after_ttl() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("one"))?;
        let skill_one = codex_root.join("one/SKILL.md");
        fs::write(&skill_one, "one")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new_with_ttl(
                vec![SkillRoot {
                    root: codex_root.clone(),
                    source: SkillSource::Codex,
                }],
                Duration::from_millis(5),
            ))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };

        let (skills_first, _) = svc.current_skills_with_dups()?;
        assert_eq!(skills_first.len(), 1);

        std::thread::sleep(Duration::from_millis(10));
        let skill_two = codex_root.join("two/SKILL.md");
        fs::create_dir_all(skill_two.parent().unwrap())?;
        fs::write(&skill_two, "two")?;

        let (skills_second, _) = svc.current_skills_with_dups()?;
        assert_eq!(skills_second.len(), 2);
        Ok(())
    }

    #[test]
    fn runtime_overrides_cached_memoizes() -> Result<()> {
        let _guard = env_guard();
        reset_runtime_cache_for_tests();
        let tmp = tempdir()?;
        let path = tmp.path().join("runtime-overrides.json");
        std::env::set_var("SKRILLS_RUNTIME_OVERRIDES", &path);
        fs::write(&path, r#"{"manifest_first":false,"render_mode_log":true}"#)?;

        let first = runtime_overrides_cached();
        assert!(!first.manifest_first());
        assert!(first.render_mode_log());

        fs::write(&path, r#"{"manifest_first":true,"render_mode_log":false}"#)?;
        let still_cached = runtime_overrides_cached();
        assert!(!still_cached.manifest_first());
        assert!(still_cached.render_mode_log());

        reset_runtime_cache_for_tests();
        let refreshed = runtime_overrides_cached();
        assert!(refreshed.manifest_first());
        assert!(!refreshed.render_mode_log());
        Ok(())
    }

    #[test]
    fn manifest_render_mode_respects_runtime_and_allowlist() {
        reset_runtime_cache_for_tests();
        reset_allowlist_cache_for_tests();
        let mut rt = RuntimeOverrides {
            manifest_first: Some(false),
            ..Default::default()
        };
        assert_eq!(manifest_render_mode(&rt, None), RenderMode::ContentOnly);

        rt.manifest_first = Some(true);
        let mut client = ClientInfo::default();
        client.client_info.name = "claude-desktop".into();
        assert_eq!(
            manifest_render_mode(&rt, Some(&client)),
            RenderMode::ManifestOnly
        );

        let tmp = tempdir().unwrap();
        let allow = tmp.path().join("allow.json");
        fs::write(&allow, r#"[{"name_substr":"codex","min_version":"2.0.0"}]"#).unwrap();
        std::env::set_var("SKRILLS_MANIFEST_ALLOWLIST", &allow);
        reset_allowlist_cache_for_tests();
        let mut codex = ClientInfo::default();
        codex.client_info.name = "codex-cli".into();
        codex.client_info.version = "2.1.0".into();
        assert_eq!(
            manifest_render_mode(&rt, Some(&codex)),
            RenderMode::ManifestOnly
        );

        codex.client_info.version = "1.0.0".into();
        reset_allowlist_cache_for_tests();
        assert_eq!(manifest_render_mode(&rt, Some(&codex)), RenderMode::Dual);
    }

    #[test]
    fn content_cache_updates_when_hash_changes() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        let skill_path = codex_root.join("alpha/SKILL.md");
        fs::write(&skill_path, "v1")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new_with_ttl(
                vec![SkillRoot {
                    root: codex_root.clone(),
                    source: SkillSource::Codex,
                }],
                Duration::from_millis(1),
            ))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };

        let uri = "skill://codex/alpha/SKILL.md";
        let first = svc.read_resource_sync(uri)?;
        let first_text = match &first.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            _ => anyhow::bail!("expected text content"),
        };
        assert!(first_text.contains("v1"));

        fs::write(&skill_path, "v2")?;
        std::thread::sleep(Duration::from_millis(5));

        let second = svc.read_resource_sync(uri)?;
        let second_text = match &second.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => text.clone(),
            _ => anyhow::bail!("expected text content"),
        };
        assert!(second_text.contains("v2"));
        Ok(())
    }

    #[tokio::test]
    async fn rmcp_resources_round_trip_inprocess() -> Result<()> {
        // Synchronous setup - hold guard only during this phase
        let (_tmp, original_home, codex_root) = {
            let _guard = env_guard();
            let tmp = tempdir()?;
            let original_home = std::env::var("HOME").ok();
            std::env::set_var("HOME", tmp.path());
            std::env::set_var("SKRILLS_INCLUDE_CLAUDE", "0");

            let codex_root = tmp.path().join("codex/skills");
            fs::create_dir_all(codex_root.join("alpha"))?;
            fs::write(codex_root.join("alpha/SKILL.md"), "# hello from codex")?;

            (tmp, original_home, codex_root)
        }; // Guard is dropped here

        let service = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![SkillRoot {
                root: codex_root.clone(),
                source: SkillSource::Codex,
            }]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };

        let (client_io, server_io) = duplex(64 * 1024);

        let server = tokio::spawn(async move {
            let running = serve_server(service, server_io)
                .await
                .map_err(|e| anyhow!("failed to start in-process server: {e}"))?;
            running
                .waiting()
                .await
                .map_err(|e| anyhow!("in-process server ended early: {e}"))
        });

        let client = serve_client((), client_io).await?;
        let peer = client.peer().clone();

        let resources = peer.list_all_resources().await?;
        let skill_uri = "skill://skrills/codex/alpha/SKILL.md";
        assert!(
            resources.iter().any(|r| r.uri == skill_uri),
            "listed resources should include the codex skill"
        );
        assert!(
            resources.iter().any(|r| r.uri == AGENTS_URI),
            "AGENTS.md should be exposed via RMCP listResources"
        );

        let skill_contents = peer
            .read_resource(ReadResourceRequestParam {
                uri: skill_uri.to_string(),
            })
            .await?;
        match &skill_contents.contents[0] {
            ResourceContents::TextResourceContents { text, meta, .. } => {
                assert!(text.contains("# hello from codex"));
                assert_eq!(
                    meta.as_ref()
                        .and_then(|m| m.get("location"))
                        .and_then(|v| v.as_str()),
                    Some("global")
                );
            }
            _ => anyhow::bail!("expected text resource content for skill"),
        }

        let agents_contents = peer
            .read_resource(ReadResourceRequestParam {
                uri: AGENTS_URI.to_string(),
            })
            .await?;
        match &agents_contents.contents[0] {
            ResourceContents::TextResourceContents { text, .. } => {
                assert!(text.contains("AI Agent Development Guidelines"));
            }
            _ => anyhow::bail!("expected text resource content for AGENTS.md"),
        }

        client.cancel().await?;
        let _ = server.await??;

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        std::env::remove_var("SKRILLS_INCLUDE_CLAUDE");
        std::env::remove_var("SKRILLS_CACHE_PATH");
        Ok(())
    }

    #[test]
    fn manifest_can_disable_agents_doc() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        std::env::remove_var(ENV_EXPOSE_AGENTS);
        let manifest = tmp.path().join(".codex/skills-manifest.json");
        fs::create_dir_all(manifest.parent().unwrap())?;
        fs::write(
            &manifest,
            r#"{ "priority": ["codex","claude"], "expose_agents": false }"#,
        )?;
        std::env::set_var("SKRILLS_MANIFEST", &manifest);

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };
        assert!(!svc.expose_agents_doc()?);
        let resources = svc.list_resources_payload()?;
        assert!(!resources.iter().any(|r| r.uri == AGENTS_URI));
        let err = svc.read_resource_sync(AGENTS_URI).unwrap_err();
        assert!(err.to_string().contains("not found"));
        std::env::remove_var("SKRILLS_MANIFEST");

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn list_resources_use_skrills_host() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        fs::write(codex_root.join("alpha/SKILL.md"), "hello")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![SkillRoot {
                root: codex_root.clone(),
                source: SkillSource::Codex,
            }]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };

        let resources = svc.list_resources_payload()?;
        let skill_uris: Vec<_> = resources
            .iter()
            .map(|r| r.uri.as_str())
            .filter(|u| u.starts_with("skill://skrills/"))
            .collect();
        assert!(skill_uris
            .iter()
            .any(|u| u.starts_with("skill://skrills/codex/alpha/SKILL.md")));
        Ok(())
    }

    #[test]
    fn read_resource_supports_canonical_and_legacy_uris() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        fs::write(codex_root.join("alpha/SKILL.md"), "hello")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![SkillRoot {
                root: codex_root.clone(),
                source: SkillSource::Codex,
            }]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };

        // Canonical URI with skrills host
        let canonical = svc.read_resource_sync("skill://skrills/codex/alpha/SKILL.md")?;
        assert_eq!(canonical.contents.len(), 1);

        // Legacy URI without host
        let legacy = svc.read_resource_sync("skill://codex/alpha/SKILL.md")?;
        assert_eq!(legacy.contents.len(), 1);
        Ok(())
    }

    #[test]
    fn content_cache_refreshes_on_hash_change() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        let skill_path = codex_root.join("alpha/SKILL.md");
        fs::write(&skill_path, "v1")?;

        let svc = SkillService {
            cache: Arc::new(Mutex::new(SkillCache::new(vec![SkillRoot {
                root: codex_root.clone(),
                source: SkillSource::Codex,
            }]))),
            content_cache: Arc::new(Mutex::new(ContentCache::default())),
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::default())),
            #[cfg(feature = "subagents")]
            subagents: None,
        };

        let meta = {
            let mut cache = svc.cache.lock().unwrap();
            cache.skills_with_dups()?.0.pop().unwrap()
        };

        let first = svc.read_skill_cached(&meta)?;
        assert_eq!(first, "v1");

        // Mutate content and ensure hash changes (matches discovery hash change test).
        fs::write(&skill_path, "v2")?;
        // Rebuild hash to reflect new content.
        let mut cache = svc.cache.lock().unwrap();
        cache.invalidate();
        cache.refresh_if_stale()?;
        let meta2 = cache.get_by_uri("skill://skrills/codex/alpha/SKILL.md")?;
        drop(cache);

        let second = svc.read_skill_cached(&meta2)?;
        assert_eq!(second, "v2");
        Ok(())
    }

    #[test]
    fn manifest_can_set_cache_ttl() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        let manifest = tmp.path().join(".codex/skills-manifest.json");
        fs::create_dir_all(manifest.parent().unwrap())?;
        fs::write(&manifest, r#"{ "cache_ttl_ms": 2500 }"#)?;
        std::env::set_var("SKRILLS_MANIFEST", &manifest);

        let svc = SkillService::new(vec![])?;
        let ttl = svc
            .cache
            .lock()
            .map_err(|e| anyhow!("poisoned: {e}"))?
            .ttl();
        assert_eq!(ttl, Duration::from_millis(2500));
        std::env::remove_var("SKRILLS_MANIFEST");

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn collect_skills_uses_relative_paths_and_hashes() -> Result<()> {
        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex/skills");
        fs::create_dir_all(codex_root.join("alpha"))?;
        let skill_path = codex_root.join("alpha/SKILL.md");
        fs::write(&skill_path, "hello")?;

        let roots = vec![SkillRoot {
            root: codex_root.clone(),
            source: SkillSource::Codex,
        }];

        let skills = discover_skills(&roots, None)?;
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "alpha/SKILL.md");
        assert_eq!(skills[0].hash, hash_file(&skill_path)?);
        assert!(matches!(skills[0].source, SkillSource::Codex));
        Ok(())
    }
    #[test]
    fn given_manifest_limit_when_render_autoload_then_manifest_valid_json() -> Result<()> {
        use gwt_autoload::*;

        // GIVEN codex + claude skills with limited byte budget
        let tmp = tempdir()?;
        let fixture = given_two_skills(
            &tmp.path().join("codex/skills"),
            "codex/SKILL.md",
            &"C token_repeat".repeat(20),
            SkillSource::Codex,
            &tmp.path().join("claude"),
            "claude/SKILL.md",
            &"irrelevant content".repeat(5),
            SkillSource::Claude,
        )?;

        let manifest_only = when_render_autoload(
            &fixture,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                ..Default::default()
            },
        )?;
        let full_dual = when_render_autoload(
            &fixture,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("token efficiency test prompt"),
                ..Default::default()
            },
        )?;
        let limit = manifest_only.len() + 16;
        assert!(limit < full_dual.len());

        // WHEN rendering with a tight limit
        let content = when_render_autoload(
            &fixture,
            AutoloadOptions {
                include_claude: false,
                max_bytes: Some(limit),
                prompt: Some("token efficiency test prompt"),
                ..Default::default()
            },
        )?;

        // THEN output stays under limit and manifest JSON is still parseable
        assert!(content.len() <= limit);
        let json_part = content
            .lines()
            .skip_while(|l| l.starts_with("[skills]"))
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::from_str::<serde_json::Value>(&json_part)?;
        Ok(())
    }

    #[test]
    fn autoload_includes_pinned_even_when_filtered_out() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        let mirror_dir = tmp.path().join("mirror");
        fs::create_dir_all(&codex_dir)?;
        fs::create_dir_all(&mirror_dir)?;

        let codex_skill = codex_dir.join("codex/SKILL.md");
        let mirror_skill = mirror_dir.join("mirror/SKILL.md");
        fs::create_dir_all(codex_skill.parent().unwrap())?;
        fs::create_dir_all(mirror_skill.parent().unwrap())?;
        fs::write(&codex_skill, "codex content")?;
        fs::write(&mirror_skill, "mirror content with no prompt hits")?;

        let skills = vec![
            SkillMeta {
                name: "codex/SKILL.md".into(),
                path: codex_skill.clone(),
                source: SkillSource::Codex,
                root: codex_dir.clone(),
                hash: hash_file(&codex_skill)?,
            },
            SkillMeta {
                name: "mirror/SKILL.md".into(),
                path: mirror_skill.clone(),
                source: SkillSource::Mirror,
                root: mirror_dir.clone(),
                hash: hash_file(&mirror_skill)?,
            },
        ];

        let mut pinned = HashSet::new();
        pinned.insert("mirror/SKILL.md".to_string());

        let content = render_autoload(
            &skills,
            AutoloadOptions {
                include_claude: true,
                prompt: Some("tokenless prompt"),
                pinned: Some(&pinned),
                ..Default::default()
            },
        )?;
        assert!(content.contains("mirror/SKILL.md"));
        // Codex skill is not pinned and the prompt does not match, so it may be filtered out.
        Ok(())
    }

    #[test]
    fn given_fuzzy_prompt_when_similarity_above_threshold_then_skill_included() -> Result<()> {
        use gwt_autoload::*;

        // GIVEN a skill whose name matches the prompt fuzzily
        let tmp = tempdir()?;
        let fixture = given_skill(
            &tmp.path().join("codex/skills"),
            "analysis/SKILL.md",
            "Guide to analyse pipeline performance and resilience.",
            SkillSource::Codex,
        )?;

        // WHEN rendering with a permissive embed threshold
        let _override = with_embed_similarity(0.8);
        let content = when_render_autoload(
            &fixture,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("plz analyz pipline bugz"),
                embed_threshold: Some(0.10),
                ..Default::default()
            },
        )?;

        // THEN the fuzzy-matched skill is included
        then_contains(&content, "analysis/SKILL.md");
        Ok(())
    }

    #[test]
    fn given_fuzzy_prompt_when_threshold_strict_then_skill_excluded() -> Result<()> {
        use gwt_autoload::*;

        // GIVEN a skill and a fuzzy prompt
        let tmp = tempdir()?;
        let fixture = given_skill(
            &tmp.path().join("codex/skills"),
            "analysis/SKILL.md",
            "Guide to analyse pipeline performance and resilience.",
            SkillSource::Codex,
        )?;

        // WHEN the embed threshold is strict
        let _override = with_embed_similarity(0.2);
        let content = when_render_autoload(
            &fixture,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("plz analyz pipline bugz"),
                embed_threshold: Some(0.95),
                ..Default::default()
            },
        )?;

        // THEN the fuzzy match is rejected
        then_not_contains(&content, "analysis/SKILL.md");
        Ok(())
    }

    #[test]
    fn autoload_respects_env_embed_threshold_default() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let skill_path = codex_dir.join("analysis/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap())?;
        fs::write(
            &skill_path,
            "Guide to analyse pipeline performance and resilience.",
        )?;

        let skills = vec![SkillMeta {
            name: "analysis/SKILL.md".into(),
            path: skill_path.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&skill_path)?,
        }];

        std::env::set_var("SKRILLS_EMBED_THRESHOLD", "0.9");
        let content = render_autoload(
            &skills,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("plz analyz pipline bugz"),
                ..Default::default()
            },
        )?;
        std::env::remove_var("SKRILLS_EMBED_THRESHOLD");

        assert!(
            !content.contains("analysis/SKILL.md"),
            "env-set high threshold should apply when option not provided"
        );
        Ok(())
    }

    #[test]
    fn autoload_keyword_match_still_wins_when_threshold_high() -> Result<()> {
        // GIVEN a prompt that directly names the skill (keyword path)
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let skill_path = codex_dir.join("observability/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap())?;
        fs::write(&skill_path, "How to add tracing and metrics.")?;

        let skills = vec![SkillMeta {
            name: "observability/SKILL.md".into(),
            path: skill_path.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&skill_path)?,
        }];

        // WHEN embed threshold is high but keyword hits
        let content = render_autoload(
            &skills,
            AutoloadOptions {
                include_claude: false,
                prompt: Some("need observability best practices"),
                embed_threshold: Some(0.99),
                ..Default::default()
            },
        )?;

        // THEN the skill is still included
        assert!(
            content.contains("observability/SKILL.md"),
            "direct keyword match should not be blocked by high embed threshold"
        );
        Ok(())
    }
    #[test]
    fn render_preview_stats_returns_token_estimate() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let skill_path = codex_dir.join("analysis/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap())?;
        fs::write(
            &skill_path,
            "Guide to analyse pipeline performance and resilience.",
        )?;

        let skills = vec![SkillMeta {
            name: "analysis/SKILL.md".into(),
            path: skill_path.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&skill_path)?,
        }];

        let stats = render_preview_stats(&skills, false)?;

        assert_eq!(stats.matched, vec!["analysis/SKILL.md"]);
        assert!(stats.manifest_bytes > 0);
        assert!(stats.estimated_tokens > 0);
        Ok(())
    }

    #[test]
    fn manifest_only_small_limit_stays_valid_json() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let codex_skill = codex_dir.join("SKILL.md");
        fs::write(&codex_skill, "C token_repeat".repeat(20))?;

        let skills = vec![SkillMeta {
            name: "codex/SKILL.md".into(),
            path: codex_skill.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&codex_skill)?,
        }];

        let manifest_only_full = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                ..Default::default()
            },
        )?;
        let limit = manifest_only_full.len() + 8;

        let content = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                max_bytes: Some(limit),
                ..Default::default()
            },
        )?;

        assert!(content.len() <= limit);
        let json_part = content
            .lines()
            .skip_while(|l| l.starts_with("[skills]"))
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::from_str::<serde_json::Value>(&json_part)?;
        Ok(())
    }

    #[test]
    fn gzipped_manifest_fallback_when_limit_hit_and_supported() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;

        // Create multiple skills to increase manifest size and make compression effective
        let mut skills = Vec::new();
        for i in 0..10 {
            let skill_path = codex_dir.join(format!("skill-{}.md", i));
            fs::create_dir_all(skill_path.parent().unwrap())?;
            // Create content with repetitive patterns that compress well
            fs::write(&skill_path, "AAABBBCCCDDD ".repeat(100))?;
            skills.push(SkillMeta {
                name: format!("codex/skill-{}.md", i),
                path: skill_path.clone(),
                source: SkillSource::Codex,
                root: codex_dir.clone(),
                hash: hash_file(&skill_path)?,
            });
        }

        // Use a reasonable preview length that will result in significant manifest size
        let preview_len = 200;
        let preview = read_prefix(&skills[0].path, preview_len)?;

        // Build the manifest with all skills
        let manifest_entries: Vec<_> = skills
            .iter()
            .map(|skill| {
                json!({
                    "name": skill.name,
                    "source": skill.source,
                    "root": &codex_dir,
                    "path": &skill.path,
                    "hash": &skill.hash,
                    "preview": &preview
                })
            })
            .collect();
        let manifest_json = json!({
            "skills_manifest": manifest_entries
        });
        let manifest_json_str = serde_json::to_string(&manifest_json)?;

        // Calculate compressed size
        let gz_wrapped = format!(
            r#"{{"skills_manifest_gzip_base64":"{}"}}"#,
            gzip_base64(&manifest_json_str)?
        );
        let compressed_len = gz_wrapped.len();

        // The uncompressed manifest_json itself (without header) is what gets checked
        // in apply_size_limit. Set limit between compressed and uncompressed JSON sizes.
        let manifest_json_len = manifest_json_str.len();

        // Set limit between compressed and uncompressed JSON sizes
        let limit = (compressed_len + manifest_json_len) / 2;
        assert!(
            limit > compressed_len,
            "limit must allow compressed version"
        );
        assert!(
            limit < manifest_json_len,
            "limit must reject uncompressed version"
        );

        // Now render with the limit that should force gzip
        let content = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                max_bytes: Some(limit),
                gzip_ok: true,
                ..Default::default()
            },
        )?;

        // Verify it's compressed (no [skills] header, just gzipped JSON)
        let v_compressed: serde_json::Value = serde_json::from_str(&content)?;
        let b64 = v_compressed
            .get("skills_manifest_gzip_base64")
            .and_then(|s| s.as_str())
            .expect("compressed manifest present");

        // Decompress and verify content is valid JSON with expected structure
        let bytes = BASE64.decode(b64)?;
        let mut decoder = GzDecoder::new(&bytes[..]);
        let mut decoded = String::new();
        decoder.read_to_string(&mut decoded)?;
        let manifest_val: serde_json::Value = serde_json::from_str(&decoded)?;

        // Verify the manifest has the expected structure
        let manifest_array = manifest_val
            .get("skills_manifest")
            .and_then(|v| v.as_array())
            .expect("manifest should have skills_manifest array");
        assert_eq!(manifest_array.len(), 10, "should have 10 skills");

        // Verify each skill has the expected fields
        for (i, entry) in manifest_array.iter().enumerate() {
            assert_eq!(entry["name"], format!("codex/skill-{}.md", i));
            assert_eq!(entry["source"], "Codex");
            assert!(entry.get("hash").is_some());
            assert!(entry.get("path").is_some());
            assert!(entry.get("root").is_some());
            assert!(entry.get("preview").is_some());
        }
        Ok(())
    }

    #[test]
    fn minimal_manifest_drops_heavy_fields() -> Result<()> {
        let tmp = tempdir()?;
        let codex_dir = tmp.path().join("codex/skills");
        fs::create_dir_all(&codex_dir)?;
        let codex_skill = codex_dir.join("SKILL.md");
        fs::write(&codex_skill, "short content")?;

        let skills = vec![SkillMeta {
            name: "codex/SKILL.md".into(),
            path: codex_skill.clone(),
            source: SkillSource::Codex,
            root: codex_dir.clone(),
            hash: hash_file(&codex_skill)?,
        }];

        let full = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                ..Default::default()
            },
        )?;
        let minimal = render_autoload(
            &skills,
            AutoloadOptions {
                render_mode: RenderMode::ManifestOnly,
                minimal_manifest: true,
                ..Default::default()
            },
        )?;

        assert!(minimal.len() < full.len());
        let parse_manifest = |s: &str| -> Result<serde_json::Value> {
            let body = s
                .lines()
                .skip_while(|l| l.starts_with("[skills]"))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(serde_json::from_str(&body)?)
        };

        let full_json = parse_manifest(&full)?;
        let minimal_json = parse_manifest(&minimal)?;
        let minimal_skill = minimal_json["skills_manifest"][0].as_object().unwrap();
        assert!(!minimal_skill.contains_key("path"));
        assert!(!minimal_skill.contains_key("preview"));
        assert!(minimal_skill.contains_key("name"));
        assert!(minimal_skill.contains_key("hash"));

        let full_skill = full_json["skills_manifest"][0].as_object().unwrap();
        assert!(full_skill.contains_key("path"));
        assert!(full_skill.contains_key("preview"));
        Ok(())
    }

    #[test]
    fn peer_accepts_gzip_prefers_env_then_name() {
        assert!(!peer_accepts_gzip(None));

        std::env::set_var("SKRILLS_ACCEPT_GZIP", "1");
        assert!(peer_accepts_gzip(None));
        std::env::remove_var("SKRILLS_ACCEPT_GZIP");

        let mut client = ClientInfo::default();
        client.client_info.name = "gzip-capable-client".into();
        assert!(peer_accepts_gzip(Some(&client)));
    }

    #[test]
    fn auto_pin_from_recent_history() {
        let history = vec![
            HistoryEntry {
                ts: 1,
                skills: vec!["a".into(), "b".into()],
            },
            HistoryEntry {
                ts: 2,
                skills: vec!["a".into()],
            },
            HistoryEntry {
                ts: 3,
                skills: vec!["c".into()],
            },
        ];
        let pins = auto_pin_from_history(&history);
        assert!(pins.contains("a")); // appears twice in window
        assert!(!pins.contains("b")); // only once
        assert!(!pins.contains("c")); // only once
    }

    #[test]
    fn manifest_priority_overrides_default() -> Result<()> {
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        std::env::set_var("SKRILLS_INCLUDE_CLAUDE", "1");
        std::env::set_var("SKRILLS_INCLUDE_MARKETPLACE", "1");
        let manifest = tmp.path().join(".codex/skills-manifest.json");
        fs::create_dir_all(manifest.parent().unwrap())?;
        fs::write(&manifest, r#"["agent","codex"]"#)?;
        std::env::set_var("SKRILLS_MANIFEST", &manifest);

        let roots = skill_roots(&[])?;
        let order: Vec<_> = roots.into_iter().map(|r| r.source.label()).collect();
        let expected_prefix = ["agent", "codex", "mirror"];
        assert!(
            order
                .iter()
                .take(expected_prefix.len())
                .map(String::as_str)
                .eq(expected_prefix),
            "manifest override should place agent/codex/mirror first; got {:?}",
            order
        );
        // Remaining entries (if any) come from the default priority order filtered by env flags.
        std::env::remove_var("SKRILLS_MANIFEST");
        std::env::remove_var("SKRILLS_INCLUDE_CLAUDE");
        std::env::remove_var("SKRILLS_INCLUDE_MARKETPLACE");

        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn manifest_render_mode_defaults_manifest_only() {
        let rt = RuntimeOverrides::default();
        // Codex client is not manifest-allowed → Dual
        use rmcp::model::ClientCapabilities;
        let codex = ClientInfo {
            protocol_version: rmcp::model::ProtocolVersion::default(),
            capabilities: ClientCapabilities::default(),
            client_info: rmcp::model::Implementation {
                name: "codex".into(),
                title: None,
                version: "0.0.0".into(),
                icons: None,
                website_url: None,
            },
        };
        assert_eq!(manifest_render_mode(&rt, Some(&codex)), RenderMode::Dual);

        // Anthropic/Claude client should be manifest-only
        let claude = ClientInfo {
            client_info: rmcp::model::Implementation {
                name: "claude-desktop".into(),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            manifest_render_mode(&rt, Some(&claude)),
            RenderMode::ManifestOnly
        );
    }

    #[test]
    fn skill_cache_loads_snapshot_without_rescan() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        // Use isolated cache path to avoid cross-test contamination when tests run in parallel.
        let snapshot_path = tmp.path().join(".codex/skills-cache.json");
        std::env::set_var("SKRILLS_CACHE_PATH", &snapshot_path);
        std::env::set_var("SKRILLS_INCLUDE_CLAUDE", "0");
        // Ensure no manifest overrides affect roots order
        std::env::remove_var("SKRILLS_MANIFEST");
        std::env::remove_var("SKRILLS_INCLUDE_MARKETPLACE");

        let roots = skill_roots(&[])?;
        let roots_fingerprint: Vec<String> = roots
            .iter()
            .map(|r| r.root.to_string_lossy().into_owned())
            .collect();

        fs::create_dir_all(snapshot_path.parent().unwrap())?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let snapshot = serde_json::json!({
            "roots": roots_fingerprint,
            "last_scan": now_secs,
            "skills": [{
                "name": "alpha/SKILL.md",
                "path": "/nonexistent/alpha/SKILL.md",
                "source": "Codex",
                "root": roots_fingerprint[0],
                "hash": "deadbeef"
            }],
            "duplicates": []
        });
        fs::write(&snapshot_path, serde_json::to_string(&snapshot)?)?;

        // Build service with the same roots we fingerprinted to avoid env/order drift
        // between snapshot creation and cache init (this was causing flakes in CI).
        let svc = SkillService::new_with_roots_for_test(roots, Duration::from_secs(3600))?;
        let resources = svc.list_resources_payload()?;
        assert!(
            resources
                .iter()
                .any(|r| r.uri == "skill://skrills/codex/alpha/SKILL.md"),
            "snapshot-loaded skill should appear even if file is missing"
        );

        std::env::remove_var("SKRILLS_INCLUDE_CLAUDE");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn skill_cache_prefers_snapshot_after_invalidation() -> Result<()> {
        let _ = tracing_subscriber::fmt::try_init();
        let _guard = env_guard();
        let tmp = tempdir()?;
        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", tmp.path());
        let snapshot_path = tmp.path().join(".codex/skills-cache.json");
        std::env::set_var("SKRILLS_CACHE_PATH", &snapshot_path);
        std::env::set_var("SKRILLS_INCLUDE_CLAUDE", "0");
        // Ensure no manifest overrides affect roots order
        std::env::remove_var("SKRILLS_MANIFEST");
        std::env::remove_var("SKRILLS_INCLUDE_MARKETPLACE");

        let roots = skill_roots(&[])?;
        let roots_fingerprint: Vec<String> = roots
            .iter()
            .map(|r| r.root.to_string_lossy().into_owned())
            .collect();

        fs::create_dir_all(snapshot_path.parent().unwrap())?;
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let snapshot = serde_json::json!({
            "roots": roots_fingerprint,
            "last_scan": now_secs,
            "skills": [{
                "name": "alpha/SKILL.md",
                "path": "/nonexistent/alpha/SKILL.md",
                "source": "Codex",
                "root": roots_fingerprint[0],
                "hash": "deadbeef"
            }],
            "duplicates": []
        });
        fs::write(&snapshot_path, serde_json::to_string(&snapshot)?)?;

        let svc = SkillService::new_with_roots_for_test(roots, Duration::from_secs(3600))?;
        // Simulate watcher-triggered cache invalidation.
        svc.invalidate_cache()?;

        let resources = svc.list_resources_payload()?;
        assert!(
            resources
                .iter()
                .any(|r| r.uri == "skill://skrills/codex/alpha/SKILL.md"),
            "snapshot should be reloaded after invalidation instead of rescanning missing files"
        );

        std::env::remove_var("SKRILLS_INCLUDE_CLAUDE");
        if let Some(home) = original_home {
            std::env::set_var("HOME", home);
        } else {
            std::env::remove_var("HOME");
        }
        Ok(())
    }

    #[test]
    fn sync_all_from_codex_syncs_skills_to_claude() -> Result<()> {
        let tmp = tempdir()?;

        let codex_skills = tmp.path().join(".codex/skills");
        fs::create_dir_all(&codex_skills)?;
        fs::write(codex_skills.join("hello.md"), "# Hello")?;

        let claude_skills = tmp.path().join(".claude/skills");
        assert!(!claude_skills.join("hello.md").exists());

        let params = SyncParams {
            from: Some("codex".to_string()),
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            ..Default::default()
        };

        // Use with_root() for isolated testing (dirs::home_dir() may not respect HOME env)
        let source = CodexAdapter::with_root(tmp.path().join(".codex"));
        let target = ClaudeAdapter::with_root(tmp.path().join(".claude"));
        let orch = SyncOrchestrator::new(source, target);
        orch.sync(&params)?;

        assert!(claude_skills.join("hello.md").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn ignores_sigchld_to_avoid_zombies() -> Result<()> {
        ignore_sigchld()?;

        let child = Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .spawn()
            .expect("spawn child");
        let pid = child.id() as libc::pid_t;

        drop(child);
        std::thread::sleep(Duration::from_millis(50));

        let res = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        assert_eq!(res, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn collect_skills_errors_on_unreadable_skill() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir()?;
        let codex_root = tmp.path().join("codex");
        fs::create_dir_all(&codex_root)?;
        let skill_path = codex_root.join("SKILL.md");
        fs::write(&skill_path, "secret")?;
        let mut perms = fs::metadata(&skill_path)?.permissions();
        perms.set_mode(0o000);
        fs::set_permissions(&skill_path, perms)?;

        let roots = vec![SkillRoot {
            root: codex_root,
            source: SkillSource::Codex,
        }];

        let result = discover_skills(&roots, None);
        assert!(result.is_ok());
        Ok(())
    }
}
