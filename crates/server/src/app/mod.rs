//! Implements primary `skrills` application functionality.
//!
//! Includes the MCP server, skill discovery, caching, and CLI.
//!
//! The `run` function initiates the server. `runtime` manages runtime options.
//! Internal components are subject to change.
//!
//! See `docs/semver-policy.md` for versioning.
//!
//! The `watch` feature enables filesystem monitoring. Build with `--no-default-features` to disable.
//!
//! On Unix, a `SIGCHLD` handler prevents zombie processes.
//! Keep this file under ~2500 LOC; split modules if needed.

mod dispatcher;
mod intelligence;
mod mcp_registry;
mod research;
mod skill_recommendations;
mod tools;

pub use dispatcher::run;
use mcp_registry::build_mcp_registry;

#[cfg(test)]
pub(crate) use dispatcher::run_sync_with_adapters;
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
use rmcp::model::{Meta, RawResource, ReadResourceResult, Resource, ResourceContents};
use serde_json::json;
use skrills_discovery::{DuplicateInfo, SkillMeta};
#[cfg(test)]
use skrills_discovery::SkillRoot;
use skrills_state::load_manifest_settings;
#[cfg(feature = "subagents")]
use skrills_subagents::SubagentService;
use std::cmp::Reverse;
use std::collections::HashMap;
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
}

/// Starts a filesystem watcher to invalidate caches on changes.
#[cfg(feature = "watch")]
pub(crate) fn start_fs_watcher(service: &SkillService) -> Result<RecommendedWatcher> {
    let cache = service.cache.clone();
    let roots = {
        let guard = cache.lock();
        guard.watched_roots()
    };

    let mut watcher = RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                cache.lock().invalidate();
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

/// Placeholder for the disabled 'watch' feature.
///
/// Returns an error if called.
#[cfg(not(feature = "watch"))]
pub(crate) fn start_fs_watcher(_service: &SkillService) -> Result<()> {
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

    /// Resolves transitive dependencies for a skill URI.
    pub(crate) fn resolve_dependencies(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.resolve_dependencies(uri)
    }

    /// Gets direct dependents for a skill URI.
    pub(crate) fn get_dependents(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.get_dependents(uri)
    }

    /// Gets transitive dependents for a skill URI.
    pub(crate) fn get_transitive_dependents(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.get_transitive_dependents(uri)
    }

    /// Computes aggregate metrics for discovered skills.
    pub(crate) fn compute_metrics(&self, include_validation: bool) -> Result<SkillMetrics> {
        use skrills_analyze::analyze_skill;
        use skrills_validate::{validate_skill, ValidationTarget};

        let (skills, _) = self.current_skills_with_dups()?;

        let mut by_source: HashMap<String, usize> = HashMap::new();
        let mut quality_high = 0usize;
        let mut quality_medium = 0usize;
        let mut quality_low = 0usize;
        let mut total_tokens = 0usize;
        let mut largest_skill: Option<SkillTokenInfo> = None;

        // Validation counters (only computed if requested)
        let mut passing = 0usize;
        let mut with_errors = 0usize;
        let mut with_warnings = 0usize;

        for meta in &skills {
            // Read skill content (before counting to ensure consistent totals)
            let content = match fs::read_to_string(&meta.path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(path = %meta.path.display(), error = %e, "Failed to read skill file");
                    continue;
                }
            };

            // Count by source (after successful read for consistent totals)
            *by_source
                .entry(meta.source.label().to_string())
                .or_default() += 1;

            // Analyze for quality and tokens
            let analysis = analyze_skill(&meta.path, &content);

            // Quality buckets
            if analysis.quality_score >= 0.8 {
                quality_high += 1;
            } else if analysis.quality_score >= 0.5 {
                quality_medium += 1;
            } else {
                quality_low += 1;
            }

            // Token stats
            total_tokens += analysis.tokens.total;
            let skill_uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
            if largest_skill
                .as_ref()
                .is_none_or(|s| analysis.tokens.total > s.tokens)
            {
                largest_skill = Some(SkillTokenInfo {
                    uri: skill_uri,
                    tokens: analysis.tokens.total,
                });
            }

            // Optional validation
            if include_validation {
                let result = validate_skill(&meta.path, &content, ValidationTarget::Both);
                if result.claude_valid && result.codex_valid {
                    passing += 1;
                } else if result.has_errors() {
                    with_errors += 1;
                } else {
                    with_warnings += 1;
                }
            }
        }

        // Compute dependency stats from the graph
        let mut cache = self.cache.lock();
        cache.ensure_fresh()?;
        let all_skills: Vec<String> = cache.skill_uris()?;

        let mut total_dependencies = 0usize;
        let mut orphan_count = 0usize;
        let mut hub_counts: Vec<(String, usize)> = Vec::new();

        for skill_uri in &all_skills {
            let deps = cache.dependencies_raw(skill_uri);
            let dependents = cache.dependents_raw(skill_uri);

            total_dependencies += deps.len();

            if deps.is_empty() && dependents.is_empty() {
                orphan_count += 1;
            }

            if !dependents.is_empty() {
                hub_counts.push((skill_uri.to_string(), dependents.len()));
            }
        }

        // Sort hubs by dependent count (descending) and take top 5
        hub_counts.sort_by_key(|b| Reverse(b.1));
        let hub_skills: Vec<HubSkill> = hub_counts
            .into_iter()
            .take(5)
            .map(|(uri, count)| HubSkill {
                uri,
                dependent_count: count,
            })
            .collect();

        let skill_count = skills.len();
        let avg_deps = if skill_count > 0 {
            total_dependencies as f64 / skill_count as f64
        } else {
            0.0
        };

        let avg_tokens = total_tokens.checked_div(skill_count).unwrap_or(0);

        let validation_summary = if include_validation {
            Some(MetricsValidationSummary {
                passing,
                with_errors,
                with_warnings,
            })
        } else {
            None
        };

        Ok(SkillMetrics {
            total_skills: skill_count,
            by_source,
            by_quality: QualityDistribution {
                high: quality_high,
                medium: quality_medium,
                low: quality_low,
            },
            dependency_stats: DependencyStats {
                total_dependencies,
                avg_per_skill: avg_deps,
                orphan_count,
                hub_skills,
            },
            token_stats: TokenStats {
                total_tokens,
                avg_per_skill: avg_tokens,
                largest_skill,
            },
            validation_summary,
        })
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

    /// Reads a resource by URI.
    pub(crate) fn read_resource_sync(&self, uri: &str) -> Result<ReadResourceResult> {
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
        let text = self.read_skill_cached(&meta)?;

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

        Ok(ReadResourceResult { contents })
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

/// Inserts location, priority rank, and role into `readResource` responses.
/// Role can be "requested" for the main resource or "dependency" for transitive dependencies.
fn text_with_location_and_role(
    text: impl Into<String>,
    uri: &str,
    source_label: Option<&str>,
    location: &str,
    role: &str,
) -> ResourceContents {
    let mut meta = Meta::new();
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
