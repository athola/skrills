//! Implements primary `skrills` application functionality.
//!
//! Includes the MCP server, skill discovery and caching. The CLI is in the
//! `skrills` crate.
//!
//! `runtime` manages runtime options.
//! Internal components are subject to change.
//!
//! See `docs/semver-policy.md` for versioning.
//!
//! The `watch` feature enables filesystem monitoring. Build with `--no-default-features` to disable.
//!
//! Keep this file under ~2500 LOC; split modules if needed.

mod intelligence;
mod mcp_registry;
mod research;
mod skill_metrics;
mod skill_recommendations;
mod tools;
pub use crate::cache::build_dependency_graph;
pub use skill_metrics::compute_skill_metrics;
pub use skill_recommendations::rank_skill_recommendations;

use mcp_registry::build_mcp_registry;

#[cfg(test)]
pub(crate) use intelligence::{resolve_project_dir, select_default_skill_root};

use crate::cache::SkillCache;
use crate::discovery::{
    priority_labels, read_skill, skill_roots, AGENTS_DESCRIPTION, AGENTS_NAME, AGENTS_TEXT,
    AGENTS_URI, ENV_EXPOSE_AGENTS,
};
// Note: skill_trace imports moved to tools.rs
use crate::mcp_gateway::{ContextStats, McpToolRegistry};
use anyhow::{anyhow, Result};
#[cfg(feature = "watch")]
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use rmcp::model::{MetaObject, ReadResourceResult, Resource, ResourceContents};
use serde_json::json;
#[cfg(test)]
use skrills_discovery::SkillRoot;
use skrills_discovery::{DuplicateInfo, SkillMeta};
use skrills_state::load_manifest_settings;
#[cfg(feature = "subagents")]
use skrills_subagents::SubagentService;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

// Re-export metrics and recommendation types from dedicated module
pub use crate::metrics_types::{
    DependencyStats, HubSkill, MetricsValidationSummary, QualityDistribution,
    RecommendationRelationship, SkillMetrics, SkillRecommendation, SkillRecommendations,
    SkillTokenInfo, TokenStats,
};

/// Manages and serves skills via RMCP.
///
/// Discovers, caches, and manages skill interactions.
/// Uses in-memory caching for performance.
pub struct SkillService {
    /// The cache for skill metadata.
    pub(crate) cache: Arc<Mutex<SkillCache>>,
    /// Optional subagent service (enabled via `subagents` feature).
    #[cfg(feature = "subagents")]
    pub(crate) subagents: Option<skrills_subagents::SubagentService>,
    /// Registry of MCP tools for context-optimized lazy loading.
    pub(crate) mcp_registry: Arc<Mutex<McpToolRegistry>>,
    /// Context usage statistics for tracking token savings.
    pub(crate) context_stats: Arc<ContextStats>,
    /// Where skill reads, validations and syncs are recorded for the
    /// dashboard. `None` records nothing.
    pub(crate) metrics: Option<Arc<skrills_metrics::MetricsCollector>>,
    /// `[serve] project_roots`: when set, a client-supplied `project_dir`
    /// must resolve inside one of these directories. `None` admits any path.
    pub(crate) project_roots: Option<Arc<Vec<PathBuf>>>,
}

/// Starts a filesystem watcher to invalidate caches on changes.
#[cfg(feature = "watch")]
pub fn start_fs_watcher(service: &SkillService) -> Result<RecommendedWatcher> {
    let cache = service.cache.clone();
    let roots = {
        let guard = cache.lock();
        guard.watched_roots()
    };

    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event {
                if invalidates_cache(&event.kind) {
                    cache.lock().invalidate();
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

/// Whether a filesystem event can change what discovery finds.
///
/// Reads are excluded: inotify reports every open and access, including the
/// server's own discovery walk, so reacting to them would invalidate the
/// cache the walk just built.
#[cfg(feature = "watch")]
pub(crate) fn invalidates_cache(kind: &notify::EventKind) -> bool {
    use notify::event::{MetadataKind, ModifyKind};
    use notify::EventKind;
    match kind {
        EventKind::Access(_) => false,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)) => false,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => true,
        // Unknown kinds may hide a real change; rescanning is the safe side.
        EventKind::Any | EventKind::Other => true,
    }
}

/// Placeholder for the disabled 'watch' feature.
///
/// Returns an error if called.
#[cfg(not(feature = "watch"))]
pub fn start_fs_watcher(_service: &SkillService) -> Result<()> {
    Err(anyhow!(
        "watch feature is disabled; rebuild with --features watch"
    ))
}

impl SkillService {
    /// Creates a new `SkillService` with a custom cache TTL.
    pub fn new_with_ttl(extra_dirs: Vec<PathBuf>, ttl: Duration) -> Result<Self> {
        let build_started = Instant::now();
        let roots = skill_roots(&extra_dirs)?;

        // Build MCP registry with all available tools
        let mcp_registry = Arc::new(Mutex::new(build_mcp_registry()));
        let context_stats = ContextStats::new();

        let elapsed_ms = build_started.elapsed().as_millis();
        tracing::info!(
            target: "skrills::startup",
            elapsed_ms,
            roots = roots.len(),
            mcp_tools = mcp_registry.lock().len(),
            skills = "deferred", // Skill discovery is deferred until after initialize to keep initial response fast.
            "SkillService constructed"
        );
        Ok(Self {
            cache: Arc::new(Mutex::new(SkillCache::new_with_ttl(roots, ttl))),
            #[cfg(feature = "subagents")]
            subagents: Some(SubagentService::new()?),
            mcp_registry,
            context_stats,
            metrics: None,
            project_roots: None,
        })
    }

    /// Records skill reads, validations and syncs to `collector`, which the
    /// dashboard's metrics API reads.
    pub fn with_metrics(mut self, collector: Arc<skrills_metrics::MetricsCollector>) -> Self {
        self.metrics = Some(collector);
        self
    }

    /// Restricts every client-supplied `project_dir` to `roots` (the
    /// `[serve] project_roots` setting). Paths are compared after
    /// canonicalizing both sides, so `..` and symlinks cannot leave a root.
    pub fn with_project_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.project_roots = Some(Arc::new(roots));
        self
    }

    /// Records to the shared on-disk store (`~/.skrills/metrics.db`), where a
    /// dashboard in another process can read it. When the store cannot be
    /// opened this logs a warning and records nothing.
    pub fn with_persistent_metrics(self) -> Self {
        match skrills_metrics::MetricsCollector::persistent_default() {
            Ok(collector) => self.with_metrics(Arc::new(collector)),
            Err(e) => {
                tracing::warn!(
                    target: "skrills::metrics",
                    error = %e,
                    "could not open ~/.skrills/metrics.db; skill usage will not be recorded"
                );
                self
            }
        }
    }

    /// Runs `record` against the collector, if any. A failed write is logged
    /// and never fails the request it describes.
    pub(crate) fn record_metric(
        &self,
        what: &str,
        record: impl FnOnce(&skrills_metrics::MetricsCollector) -> skrills_metrics::Result<()>,
    ) {
        if let Some(collector) = &self.metrics {
            if let Err(e) = record(collector) {
                tracing::warn!(target: "skrills::metrics", error = %e, what, "failed to record metric");
            }
        }
    }

    /// Test-only helper to build a service from explicit roots without
    /// re-evaluating environment-driven discovery order. This prevents tests
    /// that persist snapshots from becoming brittle when environment or
    /// priority configuration shifts between snapshot creation and service
    /// construction.
    #[cfg(test)]
    fn new_with_roots_for_test(roots: Vec<SkillRoot>, ttl: Duration) -> Result<Self> {
        let build_started = Instant::now();
        let mcp_registry = Arc::new(Mutex::new(build_mcp_registry()));
        let context_stats = ContextStats::new();
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
            #[cfg(feature = "subagents")]
            subagents: Some(SubagentService::new()?),
            mcp_registry,
            context_stats,
            metrics: None,
            project_roots: None,
        })
    }

    /// Clear the metadata and content caches.
    ///
    /// The next cache access will trigger a rescan.
    #[cfg(test)]
    fn invalidate_cache(&self) -> Result<()> {
        self.cache.lock().invalidate();
        Ok(())
    }

    /// Returns the current skills and a log of any duplicates.
    ///
    /// Duplicates are resolved by priority, retaining the winning skill.
    pub(crate) fn current_skills_with_dups(&self) -> Result<(Vec<SkillMeta>, Vec<DuplicateInfo>)> {
        let mut cache = self.cache.lock();
        cache.skills_with_dups()
    }

    /// Reports whether a skill URI is present in the cache.
    ///
    /// The refresh is explicit so a discovery failure surfaces as an error
    /// rather than as a missing skill.
    pub fn has_skill(&self, uri: &str) -> Result<bool> {
        let mut cache = self.cache.lock();
        cache.ensure_fresh()?;
        Ok(cache.skill_by_uri(uri).is_ok())
    }

    /// Resolves transitive dependencies for a skill URI.
    pub fn resolve_dependencies(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.resolve_dependencies(uri)
    }

    /// Gets direct, non-transitive dependencies for a skill URI.
    pub fn get_direct_dependencies(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.get_direct_dependencies(uri)
    }

    /// Gets direct dependents for a skill URI.
    pub fn get_dependents(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.get_dependents(uri)
    }

    /// Gets transitive dependents for a skill URI.
    pub fn get_transitive_dependents(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.get_transitive_dependents(uri)
    }

    /// Computes aggregate metrics for discovered skills: the readable ones
    /// count, against the cache's dependency graph (see
    /// [`compute_skill_metrics`]).
    pub fn compute_metrics(&self, include_validation: bool) -> Result<SkillMetrics> {
        let (skills, _) = self.current_skills_with_dups()?;
        let mut cache = self.cache.lock();
        cache.ensure_fresh()?;
        Ok(compute_skill_metrics(
            &skills,
            cache.dependency_graph(),
            include_validation,
        ))
    }

    // Note: Tool handlers (validate_skills_tool, sync_all_tool, skill_loading_status_tool,
    // enable_skill_trace_tool, disable_skill_trace_tool, skill_loading_selftest_tool)
    // are now in the `tools` submodule.

    /// Generates the MCP `listResources` payload.
    pub(crate) fn list_resources_payload(&self) -> Result<Vec<Resource>> {
        let (skills, dup_log) = self.current_skills_with_dups()?;
        let mut resources: Vec<Resource> = skills
            .into_iter()
            .map(|s| {
                let uri = format!("skill://skrills/{}/{}", s.source.label(), s.name);
                Resource::new(uri, s.name.clone())
                    .with_description(format!(
                        "Skill from {} [location: {}]",
                        s.source.label(),
                        s.source.location()
                    ))
                    .with_mime_type("text/markdown")
            })
            .collect();
        // Expose AGENTS.md guidelines as a first-class resource for clients, unless disabled.
        if self.expose_agents_doc()? {
            let agents = Resource::new(AGENTS_URI, AGENTS_NAME)
                .with_description(AGENTS_DESCRIPTION)
                .with_mime_type("text/markdown");
            resources.insert(0, agents);
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

    /// Reads a resource by URI.
    pub(crate) fn read_resource_sync(&self, uri: &str) -> Result<ReadResourceResult> {
        if uri == AGENTS_URI {
            if !self.expose_agents_doc()? {
                return Err(anyhow!("resource not found"));
            }
            return Ok(ReadResourceResult::new(vec![text_with_location(
                AGENTS_TEXT,
                uri,
                None,
                "global",
            )]));
        }
        if !uri.starts_with("skill://") {
            return Err(anyhow!("unsupported uri"));
        }

        // Parse query parameters
        let (base_uri, resolve_deps) = parse_uri_with_query(uri);

        let rest = base_uri.trim_start_matches("skill://");
        let mut parts = rest.splitn(3, '/');
        let host = parts.next().unwrap_or("");
        let first = parts.next().ok_or_else(|| anyhow!("invalid uri"))?;
        let remainder = parts.next();
        let canonical_uri = if host == "skrills" {
            let name = remainder.unwrap_or("");
            format!("skill://skrills/{}/{}", first, name)
        } else {
            // legacy: host is actually source label
            let name = if remainder.is_none() {
                first
            } else {
                &rest[host.len() + 1..]
            };
            format!("skill://{}/{}", host, name)
        };
        let meta = {
            let mut cache = self.cache.lock();
            cache.skill_by_uri(&canonical_uri)?
        };
        let read_started = Instant::now();
        let read = self.read_skill_cached(&meta);
        let elapsed_ms = u64::try_from(read_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.record_metric("skill invocation", |m| {
            m.record_skill_invocation(&meta.name, elapsed_ms, read.is_ok(), None)
        });
        let text = read?;

        let mut contents = vec![text_with_location_and_role(
            text,
            &canonical_uri,
            Some(&meta.source.label()),
            meta.source.location(),
            "requested",
        )];

        // If resolve=true, include all transitive dependencies
        if resolve_deps {
            let dep_uris = self.resolve_dependencies(&canonical_uri)?;
            for dep_uri in dep_uris {
                if let Ok(dep_meta) = {
                    let mut cache = self.cache.lock();
                    cache.skill_by_uri(&dep_uri)
                } {
                    if let Ok(dep_text) = self.read_skill_cached(&dep_meta) {
                        contents.push(text_with_location_and_role(
                            dep_text,
                            &dep_uri,
                            Some(&dep_meta.source.label()),
                            dep_meta.source.location(),
                            "dependency",
                        ));
                    }
                }
            }
        }

        Ok(ReadResourceResult::new(contents))
    }

    /// Reads skill content from disk.
    fn read_skill_cached(&self, meta: &SkillMeta) -> Result<String> {
        read_skill(&meta.path)
    }

    /// Checks if `AGENTS.md` should be exposed.
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
}

/// Parses URI and extracts query parameters.
/// Returns (base_uri, resolve_dependencies).
fn parse_uri_with_query(uri: &str) -> (&str, bool) {
    if let Some((base, query)) = uri.split_once('?') {
        let resolve = query
            .split('&')
            .any(|param| param == "resolve=true" || param == "resolve");
        (base, resolve)
    } else {
        (uri, false)
    }
}

/// Inserts location and priority rank into responses.
fn text_with_location(
    text: impl Into<String>,
    uri: &str,
    source_label: Option<&str>,
    location: &str,
) -> ResourceContents {
    let mut meta = MetaObject::new();
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

/// Inserts location, priority rank, and role into `readResource` responses.
/// Role can be "requested" for the main resource or "dependency" for transitive dependencies.
fn text_with_location_and_role(
    text: impl Into<String>,
    uri: &str,
    source_label: Option<&str>,
    location: &str,
    role: &str,
) -> ResourceContents {
    let mut meta = MetaObject::new();
    meta.insert("location".into(), json!(location));
    meta.insert("role".into(), json!(role));
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

#[cfg(test)]
mod tests;
