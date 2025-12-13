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

use crate::cli::{Cli, Commands};
use crate::commands::handle_serve_command;
use crate::discovery::{
    collect_agents, collect_skills, merge_extra_dirs, priority_labels,
    priority_labels_and_rank_map, read_skill, resolve_agent, skill_roots, AGENTS_DESCRIPTION,
    AGENTS_NAME, AGENTS_TEXT, AGENTS_URI, DEFAULT_AGENT_RUN_TEMPLATE, ENV_EXPOSE_AGENTS,
};
use crate::doctor::doctor_report;
use crate::runtime::RuntimeOverrides;
use crate::signals::ignore_sigchld;
use crate::sync::{mirror_source_root, sync_agents, sync_from_claude};
use crate::tui::tui_flow;
use anyhow::{anyhow, Result};
use clap::Parser;
#[cfg(feature = "watch")]
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, InitializeResult, ListResourcesResult,
    ListToolsResult, Meta, PaginatedRequestParam, RawResource, ReadResourceRequestParam,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, Tool, ToolAnnotations,
};
use rmcp::ServerHandler;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap};
use skrills_discovery::{discover_skills, DuplicateInfo, SkillMeta, SkillRoot};
use skrills_state::{cache_ttl, home_dir, load_manifest_settings};
#[cfg(feature = "subagents")]
use skrills_subagents::SubagentService;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
    #[allow(dead_code)]
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

    #[allow(dead_code)]
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

/// Manages and serves skills via the Remote Method Call Protocol (RMCP).
///
/// This service discovers, caches, and facilitates interaction with skills.
/// It employs in-memory caches for skill metadata and content to optimize performance.
pub(crate) struct SkillService {
    /// The cache for skill metadata.
    cache: Arc<Mutex<SkillCache>>,
    /// A flag indicating if the cache warmup has started.
    warmup_started: AtomicBool,
    /// The runtime overrides for the service.
    #[allow(dead_code)]
    runtime: Arc<Mutex<RuntimeOverrides>>,
    /// Optional subagent service (enabled via `subagents` feature).
    #[cfg(feature = "subagents")]
    subagents: Option<skrills_subagents::SubagentService>,
}

/// Start a filesystem watcher to invalidate caches when skill files change.
#[cfg(feature = "watch")]
pub(crate) fn start_fs_watcher(service: &SkillService) -> Result<RecommendedWatcher> {
    let cache = service.cache.clone();
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
pub(crate) fn start_fs_watcher(_service: &SkillService) -> Result<()> {
    Err(anyhow!(
        "watch feature is disabled; rebuild with --features watch"
    ))
}

impl SkillService {
    /// Create a new `SkillService` with the default search roots.
    #[allow(dead_code)]
    fn new(extra_dirs: Vec<PathBuf>) -> Result<Self> {
        Self::new_with_ttl(extra_dirs, cache_ttl(&load_manifest_settings))
    }

    /// Create a new `SkillService` with a custom cache TTL.
    pub(crate) fn new_with_ttl(extra_dirs: Vec<PathBuf>, ttl: Duration) -> Result<Self> {
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
    #[allow(dead_code)]
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
            warmup_started: AtomicBool::new(false),
            runtime: Arc::new(Mutex::new(RuntimeOverrides::load()?)),
            #[cfg(feature = "subagents")]
            subagents: Some(SubagentService::new()?),
        })
    }

    /// Clear the metadata and content caches.
    ///
    /// The next cache access will trigger a rescan.
    #[allow(dead_code)]
    fn invalidate_cache(&self) -> Result<()> {
        if let Ok(mut cache) = self.cache.lock() {
            cache.invalidate();
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

    /// Read the content of a skill directly from disk.
    fn read_skill_cached(&self, meta: &SkillMeta) -> Result<String> {
        read_skill(&meta.path)
    }

    /// Returns the current runtime overrides.
    #[allow(dead_code)]
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
        __context: rmcp::service::RequestContext<rmcp::RoleServer>,
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
        __context: rmcp::service::RequestContext<rmcp::RoleServer>,
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
        __context: rmcp::service::RequestContext<rmcp::RoleServer>,
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
            // Analytics tools
            Tool {
                name: "validate-skills".into(),
                title: Some("Validate skills for CLI compatibility".into()),
                description: Some(
                    "Validate skills for Claude Code and/or Codex CLI compatibility. Returns validation errors and warnings.".into(),
                ),
                input_schema: std::sync::Arc::new({
                    let mut schema = JsonMap::new();
                    schema.insert("type".into(), json!("object"));
                    schema.insert(
                        "properties".into(),
                        json!({
                            "target": {
                                "type": "string",
                                "enum": ["claude", "codex", "both"],
                                "default": "both",
                                "description": "Validation target"
                            },
                            "errors_only": {
                                "type": "boolean",
                                "default": false,
                                "description": "Only return skills with errors"
                            }
                        }),
                    );
                    schema.insert("additionalProperties".into(), json!(false));
                    schema
                }),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "analyze-skills".into(),
                title: Some("Analyze skills for token usage and optimization".into()),
                description: Some(
                    "Analyze skills for token usage, dependencies, and optimization suggestions. Returns detailed analysis with quality scores.".into(),
                ),
                input_schema: std::sync::Arc::new({
                    let mut schema = JsonMap::new();
                    schema.insert("type".into(), json!("object"));
                    schema.insert(
                        "properties".into(),
                        json!({
                            "min_tokens": {
                                "type": "integer",
                                "description": "Only include skills with at least this many tokens"
                            },
                            "include_suggestions": {
                                "type": "boolean",
                                "default": true,
                                "description": "Include optimization suggestions"
                            }
                        }),
                    );
                    schema.insert("additionalProperties".into(), json!(false));
                    schema
                }),
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
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
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
                    "sync-from-claude" => {
                        let include_marketplace = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("include_marketplace"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let home = home_dir()?;
                        let claude_root = mirror_source_root(&home);
                        let mirror_root = home.join(".codex/skills-mirror");
                        let report =
                            sync_from_claude(&claude_root, &mirror_root, include_marketplace)?;
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
                        let from = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("from"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("claude");
                        let dry_run = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("dry_run"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        let include_marketplace = request
                            .arguments
                            .as_ref()
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
                                        "Would sync {} skills from Claude to Codex",
                                        count
                                    ))],
                                    is_error: Some(false),
                                    structured_content: Some(json!({
                                        "dry_run": true,
                                        "skill_count": count
                                    })),
                                    meta: None,
                                })
                            } else {
                                let report = sync_from_claude(
                                    &claude_root,
                                    &mirror_root,
                                    include_marketplace,
                                )?;
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
                                    "Codex → Claude skill sync not yet implemented".to_string(),
                                )],
                                is_error: Some(true),
                                structured_content: None,
                                meta: None,
                            })
                        }
                    }
                    "sync-commands" => {
                        use skrills_sync::{
                            ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams,
                        };

                        let from = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("from"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("claude");
                        let dry_run = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("dry_run"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        let skip_existing_commands = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("skip_existing_commands"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        let include_marketplace = request
                            .arguments
                            .as_ref()
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
                        use skrills_sync::{
                            ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams,
                        };

                        let from = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("from"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("claude");
                        let dry_run = request
                            .arguments
                            .as_ref()
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
                        use skrills_sync::{
                            ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams,
                        };

                        let from = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("from"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("claude");
                        let dry_run = request
                            .arguments
                            .as_ref()
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
                        use skrills_sync::{
                            ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams,
                        };

                        let from = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("from"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("claude");
                        let dry_run = request
                            .arguments
                            .as_ref()
                            .and_then(|obj| obj.get("dry_run"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        let include_marketplace = request
                            .arguments
                            .as_ref()
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

                        let skip_existing_commands = request
                            .arguments
                            .as_ref()
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
                        use skrills_sync::{
                            ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams,
                        };

                        let from = request
                            .arguments
                            .as_ref()
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
                    "validate-skills" => {
                        use skrills_validate::{validate_skill, ValidationTarget as VT};

                        let args = request.arguments.clone().unwrap_or_default();
                        let target_str = args
                            .get("target")
                            .and_then(|v| v.as_str())
                            .unwrap_or("both");
                        let errors_only = args
                            .get("errors_only")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        let validation_target = match target_str {
                            "claude" => VT::Claude,
                            "codex" => VT::Codex,
                            _ => VT::Both,
                        };

                        let (skills, _) = self.current_skills_with_dups()?;
                        let mut results = Vec::new();

                        for meta in &skills {
                            let content = match fs::read_to_string(&meta.path) {
                                Ok(c) => c,
                                Err(_) => continue,
                            };
                            let result = validate_skill(&meta.path, &content, validation_target);
                            if !errors_only || result.has_errors() {
                                results.push(json!({
                                    "name": meta.name,
                                    "path": meta.path.display().to_string(),
                                    "claude_valid": result.claude_valid,
                                    "codex_valid": result.codex_valid,
                                    "errors": result.error_count(),
                                    "warnings": result.warning_count(),
                                    "issues": result.issues.iter().map(|i| json!({
                                        "severity": format!("{:?}", i.severity),
                                        "message": i.message,
                                        "line": i.line,
                                        "suggestion": i.suggestion
                                    })).collect::<Vec<_>>()
                                }));
                            }
                        }

                        let text = format!(
                            "Validated {} skills: {} Claude-valid, {} Codex-valid",
                            results.len(),
                            results
                                .iter()
                                .filter(|r| r
                                    .get("claude_valid")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false))
                                .count(),
                            results
                                .iter()
                                .filter(|r| r
                                    .get("codex_valid")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false))
                                .count()
                        );

                        Ok(CallToolResult {
                            content: vec![Content::text(text)],
                            structured_content: Some(json!({
                                "total": results.len(),
                                "target": target_str,
                                "results": results
                            })),
                            is_error: Some(false),
                            meta: None,
                        })
                    }
                    "analyze-skills" => {
                        use skrills_analyze::analyze_skill;

                        let args = request.arguments.clone().unwrap_or_default();
                        let min_tokens = args
                            .get("min_tokens")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize);
                        let include_suggestions = args
                            .get("include_suggestions")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);

                        let (skills, _) = self.current_skills_with_dups()?;
                        let mut analyses = Vec::new();

                        for meta in &skills {
                            let content = match fs::read_to_string(&meta.path) {
                                Ok(c) => c,
                                Err(_) => continue,
                            };
                            let analysis = analyze_skill(&meta.path, &content);

                            if let Some(min) = min_tokens {
                                if analysis.tokens.total < min {
                                    continue;
                                }
                            }

                            let mut result = json!({
                                "name": analysis.name,
                                "tokens": {
                                    "total": analysis.tokens.total,
                                    "frontmatter": analysis.tokens.frontmatter,
                                    "prose": analysis.tokens.prose,
                                    "code": analysis.tokens.code
                                },
                                "category": analysis.category.label(),
                                "quality_score": format!("{:.0}%", analysis.quality_score * 100.0),
                                "dependencies": {
                                    "directories": analysis.dependencies.directories,
                                    "external_urls": analysis.dependencies.external_urls().len(),
                                    "missing": analysis.dependencies.missing.len()
                                }
                            });

                            if include_suggestions && !analysis.suggestions.is_empty() {
                                result.as_object_mut().unwrap().insert(
                                    "suggestions".to_string(),
                                    json!(analysis
                                        .suggestions
                                        .iter()
                                        .map(|s| json!({
                                            "priority": format!("{:?}", s.priority),
                                            "type": format!("{:?}", s.opt_type),
                                            "message": s.message,
                                            "action": s.action
                                        }))
                                        .collect::<Vec<_>>()),
                                );
                            }

                            analyses.push(result);
                        }

                        let text = format!(
                            "Analyzed {} skills: {} total tokens",
                            analyses.len(),
                            analyses
                                .iter()
                                .filter_map(|a| a
                                    .get("tokens")
                                    .and_then(|t| t.get("total"))
                                    .and_then(|v| v.as_u64()))
                                .sum::<u64>()
                        );

                        Ok(CallToolResult {
                            content: vec![Content::text(text)],
                            structured_content: Some(json!({
                                "total": analyses.len(),
                                "analyses": analyses
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

/// Handle the `validate` command.
fn handle_validate_command(
    skill_dirs: Vec<PathBuf>,
    target: crate::cli::ValidationTarget,
    autofix: bool,
    backup: bool,
    format: String,
    errors_only: bool,
) -> Result<()> {
    use skrills_validate::{
        validate_skill, AutofixOptions, ValidationSummary, ValidationTarget as VT,
    };

    let validation_target = match target {
        crate::cli::ValidationTarget::Claude => VT::Claude,
        crate::cli::ValidationTarget::Codex => VT::Codex,
        crate::cli::ValidationTarget::Both => VT::Both,
    };

    // Collect skills from specified or default directories
    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let skills = collect_skills(&extra_dirs)?;

    if skills.is_empty() {
        println!("No skills found to validate.");
        return Ok(());
    }

    let mut results = Vec::new();
    let mut fixed_count = 0;

    for meta in skills.iter() {
        let content = match fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut result = validate_skill(&meta.path, &content, validation_target);

        // Auto-fix if requested and there are errors
        if autofix && !result.codex_valid && validation_target != VT::Claude {
            use skrills_validate::autofix_frontmatter;
            let opts = AutofixOptions {
                create_backup: backup,
                write_changes: true,
                suggested_name: Some(meta.name.clone()),
                suggested_description: None,
            };
            if let Ok(fix_result) = autofix_frontmatter(&meta.path, &content, &opts) {
                if fix_result.modified {
                    fixed_count += 1;
                    // Re-validate after fix
                    let new_content = fs::read_to_string(&meta.path)?;
                    result = validate_skill(&meta.path, &new_content, validation_target);
                }
            }
        }

        // Filter if errors_only
        if !errors_only || result.has_errors() {
            results.push(result);
        }
    }

    // Output results
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        let summary = ValidationSummary::from_results(&results);
        println!(
            "Validated {} skills: {} Claude-valid, {} Codex-valid, {} both-valid",
            summary.total, summary.claude_valid, summary.codex_valid, summary.both_valid
        );
        if fixed_count > 0 {
            println!("Auto-fixed {} skills", fixed_count);
        }
        if summary.error_count > 0 {
            println!("\nErrors ({}):", summary.error_count);
            for result in &results {
                for issue in &result.issues {
                    if issue.severity == skrills_validate::Severity::Error {
                        println!(
                            "  {} ({}): {}",
                            result.name,
                            result.path.display(),
                            issue.message
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

/// Handle the `analyze` command.
fn handle_analyze_command(
    skill_dirs: Vec<PathBuf>,
    format: String,
    min_tokens: Option<usize>,
    suggestions: bool,
) -> Result<()> {
    use skrills_analyze::{analyze_skill, AnalysisSummary, Priority};

    // Collect skills from specified or default directories
    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let skills = collect_skills(&extra_dirs)?;

    if skills.is_empty() {
        println!("No skills found to analyze.");
        return Ok(());
    }

    let mut analyses = Vec::new();

    for meta in skills.iter() {
        let content = match fs::read_to_string(&meta.path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let analysis = analyze_skill(&meta.path, &content);

        // Filter by min_tokens if specified
        if let Some(min) = min_tokens {
            if analysis.tokens.total < min {
                continue;
            }
        }

        analyses.push(analysis);
    }

    // Output results
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(&analyses)?);
    } else {
        let summary = AnalysisSummary::from_analyses(&analyses);
        println!(
            "Analyzed {} skills: {} total tokens",
            summary.total_skills, summary.total_tokens
        );
        println!(
            "Size distribution: {} small, {} medium, {} large, {} very-large",
            summary.by_category.small,
            summary.by_category.medium,
            summary.by_category.large,
            summary.by_category.very_large
        );
        println!("Average quality score: {:.0}%", summary.avg_quality * 100.0);

        if suggestions && summary.high_priority_count > 0 {
            println!(
                "\nHigh-priority suggestions ({}):",
                summary.high_priority_count
            );
            for analysis in &analyses {
                for suggestion in &analysis.suggestions {
                    if suggestion.priority == Priority::High {
                        println!("  {} - {}", analysis.name, suggestion.message);
                    }
                }
            }
        }
    }

    Ok(())
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
            validate: _validate,
            autofix: _autofix,
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
        Commands::Validate {
            skill_dirs,
            target,
            autofix,
            backup,
            format,
            errors_only,
        } => handle_validate_command(skill_dirs, target, autofix, backup, format, errors_only),
        Commands::Analyze {
            skill_dirs,
            format,
            min_tokens,
            suggestions,
        } => handle_analyze_command(skill_dirs, format, min_tokens, suggestions),
    }
}
