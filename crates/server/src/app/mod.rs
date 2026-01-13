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
//! Keep this file under ~2500 LOC; split it into modules before crossing the limit.

mod intelligence;
mod tools;

#[cfg(test)]
pub(crate) use intelligence::{resolve_project_dir, select_default_skill_root};

use crate::cache::SkillCache;
use crate::cli::{Cli, Commands};
use crate::commands::{
    handle_agent_command, handle_analyze_command, handle_analyze_project_context_command,
    handle_create_skill_command, handle_export_analytics_command, handle_import_analytics_command,
    handle_metrics_command, handle_mirror_command, handle_recommend_command,
    handle_recommend_skills_smart_command, handle_resolve_dependencies_command,
    handle_search_skills_github_command, handle_serve_command, handle_setup_command,
    handle_suggest_new_skills_command, handle_sync_agents_command, handle_sync_command,
    handle_validate_command,
};
use crate::discovery::{
    merge_extra_dirs, priority_labels, read_skill, skill_roots, AGENTS_DESCRIPTION, AGENTS_NAME,
    AGENTS_TEXT, AGENTS_URI, ENV_EXPOSE_AGENTS,
};
use crate::doctor::doctor_report;
use crate::signals::ignore_sigchld;
// Note: skill_trace imports moved to tools.rs
use crate::mcp_gateway::{ContextStats, McpToolEntry, McpToolRegistry};
use crate::sync::mirror_source_root;
use crate::tool_schemas;
use crate::tui::tui_flow;
use anyhow::{anyhow, Result};
use clap::Parser;
#[cfg(feature = "watch")]
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use rmcp::model::{Meta, RawResource, ReadResourceResult, Resource, ResourceContents};
use serde_json::json;
use skrills_discovery::{DuplicateInfo, SkillMeta, SkillRoot};
use skrills_state::{cache_ttl, home_dir, load_manifest_settings};
#[cfg(feature = "subagents")]
use skrills_subagents::SubagentService;
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

/// Manages and serves skills via the Remote Method Call Protocol (RMCP).
///
/// This service discovers, caches, and facilitates interaction with skills.
/// It employs in-memory caches for skill metadata and content to optimize performance.
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

/// Start a filesystem watcher to invalidate caches when skill files change.
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

/// This function serves as a placeholder for the filesystem watcher when the 'watch' feature is disabled.
///
/// It returns an error if called.
#[cfg(not(feature = "watch"))]
pub(crate) fn start_fs_watcher(_service: &SkillService) -> Result<()> {
    Err(anyhow!(
        "watch feature is disabled; rebuild with --features watch"
    ))
}

/// Category classification for MCP tools.
///
/// Used for organizing and filtering tools by their primary purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolCategory {
    Sync,
    Validation,
    Trace,
    Intelligence,
    Metrics,
    Dependency,
    Gateway,
}

impl ToolCategory {
    /// Infer category from a tool name using prefix/substring matching.
    fn from_tool_name(name: &str) -> Option<Self> {
        match name {
            n if n.starts_with("sync") => Some(Self::Sync),
            n if n.starts_with("validate") || n.starts_with("analyze") => Some(Self::Validation),
            n if n.contains("trace") || n.contains("instrument") => Some(Self::Trace),
            n if n.contains("recommend") || n.contains("suggest") => Some(Self::Intelligence),
            n if n.contains("metric") => Some(Self::Metrics),
            n if n.contains("depend") => Some(Self::Dependency),
            _ => None,
        }
    }

    /// Convert to a string representation for serialization.
    const fn as_str(&self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Validation => "validation",
            Self::Trace => "trace",
            Self::Intelligence => "intelligence",
            Self::Metrics => "metrics",
            Self::Dependency => "dependency",
            Self::Gateway => "gateway",
        }
    }
}

/// Build the MCP tool registry from all available tool definitions.
fn build_mcp_registry() -> McpToolRegistry {
    use crate::mcp_gateway::estimate_tokens;

    let mut registry = McpToolRegistry::new();

    // Register all internal tools from tool_schemas
    for tool in tool_schemas::all_tools() {
        let schema_json = serde_json::to_string(&tool.input_schema).unwrap_or_default();
        let estimated_tokens = estimate_tokens(&schema_json);

        // Infer category from tool name using enum matching
        let category = ToolCategory::from_tool_name(&tool.name).map(|c| c.as_str().to_string());

        registry.register(McpToolEntry {
            name: tool.name.to_string(),
            description: tool.description.clone().unwrap_or_default().to_string(),
            source: "skrills".to_string(),
            estimated_tokens,
            category,
        });
    }

    // Register gateway tools themselves
    for tool in crate::mcp_gateway::mcp_gateway_tools() {
        let schema_json = serde_json::to_string(&tool.input_schema).unwrap_or_default();
        let estimated_tokens = estimate_tokens(&schema_json);
        registry.register(McpToolEntry {
            name: tool.name.to_string(),
            description: tool.description.clone().unwrap_or_default().to_string(),
            source: "gateway".to_string(),
            estimated_tokens,
            category: Some(ToolCategory::Gateway.as_str().to_string()),
        });
    }

    registry
}

impl SkillService {
    /// Create a new `SkillService` with the default search roots.
    #[allow(dead_code)]
    fn new(extra_dirs: Vec<PathBuf>) -> Result<Self> {
        Self::new_with_ttl(extra_dirs, cache_ttl(&load_manifest_settings))
    }

    /// Create a new `SkillService` with a custom cache TTL.
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
    #[allow(dead_code)]
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
    #[allow(dead_code)]
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

    /// Resolve transitive dependencies for a skill URI.
    pub(crate) fn resolve_dependencies(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.resolve_dependencies(uri)
    }

    /// Get skills that directly depend on the given skill URI.
    pub(crate) fn get_dependents(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.get_dependents(uri)
    }

    /// Get all skills that transitively depend on the given skill URI.
    pub(crate) fn get_transitive_dependents(&self, uri: &str) -> Result<Vec<String>> {
        let mut cache = self.cache.lock();
        cache.get_transitive_dependents(uri)
    }

    /// Get skill recommendations based on dependency relationships.
    ///
    /// The algorithm:
    /// 1. Get direct dependencies of the skill (skills it needs)
    /// 2. Get direct dependents (skills that need it)
    /// 3. Find sibling skills (share common dependencies)
    /// 4. Rank by relationship type and optionally quality score
    pub(crate) fn recommend_skills(
        &self,
        uri: &str,
        limit: usize,
        include_quality: bool,
    ) -> Result<SkillRecommendations> {
        use skrills_analyze::analyze_skill;
        use std::collections::HashSet;

        let mut cache = self.cache.lock();
        cache.ensure_fresh()?;

        // Collect all skill URIs for sibling detection
        let all_uris = cache.skill_uris()?;

        // Validate that the requested URI exists
        if !all_uris.contains(&uri.to_string()) {
            anyhow::bail!("Skill not found: {}", uri);
        }

        // Get direct relationships
        let dependencies: Vec<String> = cache.dependencies_raw(uri);
        let dependents: Vec<String> = cache.dependents_raw(uri);

        // Find siblings (skills that share dependencies with this skill)
        let source_deps: HashSet<_> = dependencies.iter().cloned().collect();
        let mut siblings: Vec<String> = Vec::new();

        if !source_deps.is_empty() {
            for other_uri in &all_uris {
                if other_uri == uri {
                    continue;
                }
                // Skip if already in dependencies or dependents
                if dependencies.contains(other_uri) || dependents.contains(other_uri) {
                    continue;
                }
                let other_deps: HashSet<_> =
                    cache.dependencies_raw(other_uri).into_iter().collect();
                // Check for shared dependencies
                if !source_deps.is_disjoint(&other_deps) {
                    siblings.push(other_uri.clone());
                }
            }
        }

        // Build recommendations with scores
        let mut recommendations: Vec<SkillRecommendation> = Vec::new();

        // Dependencies get highest base score (most immediately useful)
        for dep_uri in &dependencies {
            let mut rec = SkillRecommendation {
                uri: dep_uri.clone(),
                relationship: RecommendationRelationship::Dependency,
                quality_score: None,
                score: 3.0, // Base score for dependencies
            };

            if include_quality {
                match cache.skill_by_uri(dep_uri) {
                    Ok(meta) => match fs::read_to_string(&meta.path) {
                        Ok(content) => {
                            let analysis = analyze_skill(&meta.path, &content);
                            rec.quality_score = Some(analysis.quality_score);
                            rec.score += analysis.quality_score; // Add quality bonus
                        }
                        Err(e) => {
                            tracing::warn!(uri = %dep_uri, error = %e, "Failed to read skill for quality scoring");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(uri = %dep_uri, error = %e, "Failed to find skill metadata for quality scoring");
                    }
                }
            }

            recommendations.push(rec);
        }

        // Dependents get medium base score
        for dep_uri in &dependents {
            let mut rec = SkillRecommendation {
                uri: dep_uri.clone(),
                relationship: RecommendationRelationship::Dependent,
                quality_score: None,
                score: 2.0, // Base score for dependents
            };

            if include_quality {
                match cache.skill_by_uri(dep_uri) {
                    Ok(meta) => match fs::read_to_string(&meta.path) {
                        Ok(content) => {
                            let analysis = analyze_skill(&meta.path, &content);
                            rec.quality_score = Some(analysis.quality_score);
                            rec.score += analysis.quality_score;
                        }
                        Err(e) => {
                            tracing::warn!(uri = %dep_uri, error = %e, "Failed to read skill for quality scoring");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(uri = %dep_uri, error = %e, "Failed to find skill metadata for quality scoring");
                    }
                }
            }

            recommendations.push(rec);
        }

        // Siblings get lowest base score
        for sib_uri in &siblings {
            let mut rec = SkillRecommendation {
                uri: sib_uri.clone(),
                relationship: RecommendationRelationship::Sibling,
                quality_score: None,
                score: 1.0, // Base score for siblings
            };

            if include_quality {
                match cache.skill_by_uri(sib_uri) {
                    Ok(meta) => match fs::read_to_string(&meta.path) {
                        Ok(content) => {
                            let analysis = analyze_skill(&meta.path, &content);
                            rec.quality_score = Some(analysis.quality_score);
                            rec.score += analysis.quality_score;
                        }
                        Err(e) => {
                            tracing::warn!(uri = %sib_uri, error = %e, "Failed to read skill for quality scoring");
                        }
                    },
                    Err(e) => {
                        tracing::warn!(uri = %sib_uri, error = %e, "Failed to find skill metadata for quality scoring");
                    }
                }
            }

            recommendations.push(rec);
        }

        // Sort by score descending
        recommendations.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let total_found = recommendations.len();

        // Apply limit
        recommendations.truncate(limit);

        Ok(SkillRecommendations {
            source_uri: uri.to_string(),
            total_found,
            recommendations,
        })
    }

    /// Compute aggregate metrics for all discovered skills.
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
        hub_counts.sort_by(|a, b| b.1.cmp(&a.1));
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

        let avg_tokens = if skill_count > 0 {
            total_tokens / skill_count
        } else {
            0
        };

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

    /// Generate the MCP `listResources` payload.
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

    /// Read a resource by its URI and return its content.
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

    /// Read the content of a skill directly from disk.
    fn read_skill_cached(&self, meta: &SkillMeta) -> Result<String> {
        read_skill(&meta.path)
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
}

/// Parse URI and extract query parameters.
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
        http: None,
        list_tools: false,
        auth_token: None,
        tls_cert: None,
        tls_key: None,
        cors_origins: Vec::new(),
    }) {
        Commands::Serve {
            skill_dirs,
            cache_ttl_ms,
            trace_wire,
            #[cfg(feature = "watch")]
            watch,
            http,
            list_tools,
            auth_token,
            tls_cert,
            tls_key,
            cors_origins,
        } => handle_serve_command(
            skill_dirs,
            cache_ttl_ms,
            trace_wire,
            #[cfg(feature = "watch")]
            watch,
            http,
            list_tools,
            auth_token,
            tls_cert,
            tls_key,
            cors_origins,
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
                let codex_skills_root = home.join(".codex/skills");
                let skill_report = crate::sync::sync_skills_only_from_claude(
                    &claude_root,
                    &codex_skills_root,
                    include_marketplace,
                )?;
                let _ = crate::setup::ensure_codex_skills_feature_enabled(
                    &home.join(".codex/config.toml"),
                );
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
        Commands::Metrics {
            skill_dirs,
            format,
            include_validation,
        } => handle_metrics_command(skill_dirs, format, include_validation),
        Commands::Recommend {
            uri,
            skill_dirs,
            format,
            limit,
            include_quality,
        } => handle_recommend_command(uri, skill_dirs, format, limit, include_quality),
        Commands::ResolveDependencies {
            uri,
            skill_dirs,
            direction,
            transitive,
            format,
        } => handle_resolve_dependencies_command(uri, skill_dirs, direction, transitive, format),
        Commands::RecommendSkillsSmart {
            uri,
            prompt,
            project_dir,
            limit,
            include_usage,
            include_context,
            format,
            skill_dirs,
        } => handle_recommend_skills_smart_command(
            uri,
            prompt,
            project_dir,
            limit,
            include_usage,
            include_context,
            format,
            skill_dirs,
        ),
        Commands::AnalyzeProjectContext {
            project_dir,
            include_git,
            commit_limit,
            format,
        } => handle_analyze_project_context_command(project_dir, include_git, commit_limit, format),
        Commands::SuggestNewSkills {
            project_dir,
            focus_areas,
            format,
            skill_dirs,
        } => handle_suggest_new_skills_command(project_dir, focus_areas, format, skill_dirs),
        Commands::CreateSkill {
            name,
            description,
            method,
            target_dir,
            project_dir,
            dry_run,
            format,
        } => handle_create_skill_command(
            name,
            description,
            method,
            target_dir,
            project_dir,
            dry_run,
            format,
        ),
        Commands::SearchSkillsGithub {
            query,
            limit,
            format,
        } => handle_search_skills_github_command(query, limit, format),
        Commands::ExportAnalytics {
            output,
            force_rebuild,
            format,
        } => handle_export_analytics_command(output, force_rebuild, format),
        Commands::ImportAnalytics { input, overwrite } => {
            handle_import_analytics_command(input, overwrite)
        }
    }
}

#[cfg(test)]
mod tests;
