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

use crate::cli::{Cli, Commands};
use crate::commands::{
    handle_agent_command, handle_analyze_command, handle_metrics_command, handle_mirror_command,
    handle_serve_command, handle_setup_command, handle_sync_agents_command, handle_sync_command,
    handle_validate_command,
};
use crate::discovery::{
    merge_extra_dirs, priority_labels, read_skill, skill_roots, AGENTS_DESCRIPTION, AGENTS_NAME,
    AGENTS_TEXT, AGENTS_URI, ENV_EXPOSE_AGENTS,
};
use crate::doctor::doctor_report;
use crate::signals::ignore_sigchld;
use crate::skill_trace::{self, ClientTarget as TraceTarget, TraceInstallOptions};
use crate::sync::mirror_source_root;
use crate::tui::tui_flow;
use anyhow::{anyhow, Result};
use clap::Parser;
#[cfg(feature = "watch")]
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use rmcp::model::{
    CallToolResult, Content, Meta, RawResource, ReadResourceResult, Resource, ResourceContents,
};
use serde_json::{json, Map as JsonMap, Value};
use crate::cache::SkillCache;
use skrills_discovery::{DuplicateInfo, SkillMeta, SkillRoot};
use skrills_state::{cache_ttl, home_dir, load_manifest_settings};
#[cfg(feature = "subagents")]
use skrills_subagents::SubagentService;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Re-export metrics types from dedicated module
pub use crate::metrics_types::{
    DependencyStats, HubSkill, MetricsValidationSummary, QualityDistribution, SkillMetrics,
    SkillTokenInfo, TokenStats,
};

/// Manages and serves skills via the Remote Method Call Protocol (RMCP).
///
/// This service discovers, caches, and facilitates interaction with skills.
/// It employs in-memory caches for skill metadata and content to optimize performance.
pub(crate) struct SkillService {
    /// The cache for skill metadata.
    pub(crate) cache: Arc<Mutex<SkillCache>>,
    /// Optional subagent service (enabled via `subagents` feature).
    #[cfg(feature = "subagents")]
    pub(crate) subagents: Option<skrills_subagents::SubagentService>,
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
            #[cfg(feature = "subagents")]
            subagents: Some(SubagentService::new()?),
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
            if largest_skill.is_none()
                || analysis.tokens.total > largest_skill.as_ref().unwrap().tokens
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

    pub(crate) fn validate_skills_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        use skrills_validate::{
            autofix_frontmatter, validate_skill, AutofixOptions, ValidationTarget as VT,
        };

        let target_str = args
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("both");
        let errors_only = args
            .get("errors_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let autofix = args
            .get("autofix")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let check_dependencies = args
            .get("check_dependencies")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let validation_target = match target_str {
            "claude" => VT::Claude,
            "codex" => VT::Codex,
            _ => VT::Both,
        };

        let (skills, _) = self.current_skills_with_dups()?;

        // Build a set of all valid skill URIs for dependency checking
        let valid_skill_uris: std::collections::HashSet<String> = skills
            .iter()
            .map(|s| format!("skill://skrills/{}/{}", s.source.label(), s.name))
            .collect();

        let mut results = Vec::new();
        let mut autofixed = 0usize;
        let mut total_dep_issues = 0usize;

        for meta in &skills {
            let mut content = match fs::read_to_string(&meta.path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut result = validate_skill(&meta.path, &content, validation_target);
            let mut autofixed_skill = false;

            if autofix && !result.codex_valid && validation_target != VT::Claude {
                let opts = AutofixOptions {
                    create_backup: false,
                    write_changes: true,
                    suggested_name: Some(meta.name.clone()),
                    suggested_description: None,
                };
                if let Ok(fix_result) = autofix_frontmatter(&meta.path, &content, &opts) {
                    if fix_result.modified {
                        autofixed += 1;
                        autofixed_skill = true;
                        content = fs::read_to_string(&meta.path).unwrap_or(content);
                        result = validate_skill(&meta.path, &content, validation_target);
                    }
                }
            }

            // Check dependencies if requested
            let mut dependency_issues = Vec::new();
            let mut dependency_count = 0usize;
            let mut missing_count = 0usize;

            if check_dependencies {
                let dep_analysis = skrills_analyze::analyze_dependencies(&meta.path, &content);
                dependency_count = dep_analysis.dependencies.len();

                // Check for missing local dependencies
                for missing_dep in &dep_analysis.missing {
                    let issue_type = match missing_dep.dep_type {
                        skrills_analyze::DependencyType::Module => "missing_module",
                        skrills_analyze::DependencyType::Reference => "missing_reference",
                        skrills_analyze::DependencyType::Script => "missing_script",
                        skrills_analyze::DependencyType::Asset => "missing_asset",
                        _ => "missing_file",
                    };
                    dependency_issues.push(json!({
                        "type": issue_type,
                        "target": missing_dep.target,
                        "line": missing_dep.line
                    }));
                    missing_count += 1;
                }

                // Check for unresolved skill dependencies
                let skill_uri = format!("skill://skrills/{}/{}", meta.source.label(), meta.name);
                if let Ok(deps) = self.resolve_dependencies(&skill_uri) {
                    for dep_uri in deps {
                        // Check if the dependency exists in our valid skills set
                        if !valid_skill_uris.contains(&dep_uri) {
                            dependency_issues.push(json!({
                                "type": "unresolved_skill",
                                "target": dep_uri
                            }));
                            missing_count += 1;
                        }
                    }
                }

                total_dep_issues += dependency_issues.len();
            }

            if !errors_only || result.has_errors() || !dependency_issues.is_empty() {
                let mut skill_json = json!({
                    "name": meta.name,
                    "path": meta.path.display().to_string(),
                    "claude_valid": result.claude_valid,
                    "codex_valid": result.codex_valid,
                    "errors": result.error_count(),
                    "warnings": result.warning_count(),
                    "autofixed": autofixed_skill,
                    "issues": result.issues.iter().map(|i| json!({
                        "severity": format!("{:?}", i.severity),
                        "message": i.message,
                        "line": i.line,
                        "suggestion": i.suggestion
                    })).collect::<Vec<_>>()
                });

                if check_dependencies {
                    skill_json
                        .as_object_mut()
                        .unwrap()
                        .insert("dependency_issues".to_string(), json!(dependency_issues));
                    skill_json
                        .as_object_mut()
                        .unwrap()
                        .insert("dependency_count".to_string(), json!(dependency_count));
                    skill_json
                        .as_object_mut()
                        .unwrap()
                        .insert("missing_count".to_string(), json!(missing_count));
                }

                results.push(skill_json);
            }
        }

        let text = {
            let mut base = format!(
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
            if autofixed > 0 {
                base = format!("{base}\nAuto-fixed {autofixed} skills");
            }
            if check_dependencies && total_dep_issues > 0 {
                base = format!("{base}\nFound {total_dep_issues} dependency issues");
            }
            base
        };

        let mut structured = json!({
            "total": results.len(),
            "target": target_str,
            "autofix": autofix,
            "autofixed": autofixed,
            "results": results
        });

        if check_dependencies {
            structured
                .as_object_mut()
                .unwrap()
                .insert("check_dependencies".to_string(), json!(true));
            structured.as_object_mut().unwrap().insert(
                "total_dependency_issues".to_string(),
                json!(total_dep_issues),
            );
        }

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(structured),
            is_error: Some(false),
            meta: None,
        })
    }

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

    pub(crate) fn sync_all_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        use skrills_sync::{
            parse_direction, ClaudeAdapter, CodexAdapter, SyncDirection, SyncOrchestrator,
            SyncParams,
        };

        let from = args
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("claude");
        let direction = parse_direction(from)?;
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let include_marketplace = args
            .get("include_marketplace")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Sync skills first (Codex discovery root).
        let skill_report = match direction {
            SyncDirection::ClaudeToCodex if !dry_run => {
                let home = home_dir()?;
                let claude_root = mirror_source_root(&home);
                let codex_skills_root = home.join(".codex/skills");
                let report = crate::sync::sync_skills_only_from_claude(
                    &claude_root,
                    &codex_skills_root,
                    include_marketplace,
                )?;
                let _ = crate::setup::ensure_codex_skills_feature_enabled(
                    &home.join(".codex/config.toml"),
                );
                report
            }
            _ => crate::sync::SyncReport::default(),
        };

        let skip_existing_commands = args
            .get("skip_existing_commands")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let params = SyncParams {
            from: Some(from.to_string()),
            dry_run,
            sync_commands: true,
            skip_existing_commands,
            sync_mcp_servers: true,
            sync_preferences: true,
            sync_skills: false, // Skills are handled above for Claude→Codex.
            include_marketplace,
            ..Default::default()
        };

        let report = match direction {
            SyncDirection::ClaudeToCodex => {
                let source = ClaudeAdapter::new()?;
                let target = CodexAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            }
            SyncDirection::CodexToClaude => {
                let source = CodexAdapter::new()?;
                let target = ClaudeAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            }
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

    pub(crate) fn parse_trace_target(args: &JsonMap<String, Value>) -> TraceTarget {
        match args
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("both")
        {
            "claude" => TraceTarget::Claude,
            "codex" => TraceTarget::Codex,
            _ => TraceTarget::Both,
        }
    }

    pub(crate) fn skill_loading_status_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        let target = Self::parse_trace_target(&args);
        let opts = TraceInstallOptions {
            include_cache: args
                .get("include_cache")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            include_marketplace: args
                .get("include_marketplace")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            include_mirror: args
                .get("include_mirror")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            include_agent: args
                .get("include_agent")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            ..Default::default()
        };

        let home = home_dir()?;
        let status = skill_trace::status(&home, target, &opts)?;

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "Skill loading status: found {} skill files; markers in {} files",
                status.skill_files_found, status.instrumented_markers_found
            ))],
            structured_content: Some(serde_json::to_value(status)?),
            is_error: Some(false),
            meta: None,
        })
    }

    pub(crate) fn enable_skill_trace_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        let target = Self::parse_trace_target(&args);
        let opts = TraceInstallOptions {
            instrument: args
                .get("instrument")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            backup: args.get("backup").and_then(|v| v.as_bool()).unwrap_or(true),
            dry_run: args
                .get("dry_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            include_cache: args
                .get("include_cache")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            include_marketplace: args
                .get("include_marketplace")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            include_mirror: args
                .get("include_mirror")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            include_agent: args
                .get("include_agent")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        };

        let home = home_dir()?;
        let report = skill_trace::enable_trace(&home, target, opts)?;

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "Enabled skill trace{}: installed trace={}, probe={}, instrumented={} (skipped={})",
                if report.warnings.iter().any(|w| w.contains("failed to read")) {
                    " (with warnings)"
                } else {
                    ""
                },
                report.installed_trace_skill,
                report.installed_probe_skill,
                report.instrumented_files,
                report.skipped_files
            ))],
            structured_content: Some(serde_json::to_value(report)?),
            is_error: Some(false),
            meta: None,
        })
    }

    pub(crate) fn disable_skill_trace_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        let target = Self::parse_trace_target(&args);
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let home = home_dir()?;
        let removed = skill_trace::disable_trace(&home, target, dry_run)?;

        Ok(CallToolResult {
            content: vec![Content::text(format!(
                "{} trace/probe skill directories",
                if dry_run { "Would remove" } else { "Removed" }
            ))],
            structured_content: Some(json!({ "dry_run": dry_run, "removed": removed })),
            is_error: Some(false),
            meta: None,
        })
    }

    pub(crate) fn skill_loading_selftest_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        let target = Self::parse_trace_target(&args);
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let home = home_dir()?;
        let installed = skill_trace::ensure_probe(&home, target, dry_run)?;
        let token = {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("{:x}", now)
        };

        Ok(CallToolResult {
            content: vec![Content::text(
                "Skill selftest prepared. Send the probe line shown in structured_content.",
            )],
            structured_content: Some(json!({
                "target": target,
                "probe_skill_installed": installed,
                "probe_line": format!("SKRILLS_PROBE:{token}"),
                "expected_response": format!("SKRILLS_PROBE_OK:{token}"),
                "notes": [
                    "If the probe skill was just installed, you may need to restart the Claude/Codex session for skills to reload.",
                    "If you also enabled skill tracing, every assistant response will end with a SKRILLS_SKILLS_LOADED footer."
                ]
            })),
            is_error: Some(false),
            meta: None,
        })
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn validate_skills_tool_autofix_adds_frontmatter() {
        let _guard = crate::test_support::env_guard();
        let temp = tempdir().unwrap();
        let skill_dir = temp.path().join("skills");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(&skill_path, "A skill without frontmatter").unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", temp.path());

        let service =
            SkillService::new_with_ttl(vec![skill_dir.clone()], Duration::from_secs(1)).unwrap();
        let result = service
            .validate_skills_tool(
                json!({"target": "codex", "autofix": true})
                    .as_object()
                    .cloned()
                    .unwrap(),
            )
            .unwrap();

        match original_home {
            Some(val) => std::env::set_var("HOME", val),
            None => std::env::remove_var("HOME"),
        }

        let content = std::fs::read_to_string(&skill_path).unwrap();
        assert!(
            content.starts_with("---"),
            "autofix should add frontmatter to skill files"
        );
        let structured = result.structured_content.unwrap();
        assert_eq!(
            structured.get("autofixed").and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[test]
    fn test_dependency_graph_integration() {
        use skrills_discovery::SkillRoot;

        // Initialize tracing for test
        let _ = tracing_subscriber::fmt()
            .with_env_filter("skrills::deps=debug")
            .try_init();

        let temp = tempdir().unwrap();
        let skills_dir = temp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create skill A (depends on B and C)
        let skill_a_dir = skills_dir.join("skill-a");
        fs::create_dir_all(&skill_a_dir).unwrap();
        fs::write(
            skill_a_dir.join("SKILL.md"),
            r#"---
name: skill-a
description: Skill A depends on B and C
---
# Skill A
See [skill-b](../skill-b/SKILL.md) and [skill-c](../skill-c/SKILL.md) for details.
"#,
        )
        .unwrap();

        // Create skill B (depends on D)
        let skill_b_dir = skills_dir.join("skill-b");
        fs::create_dir_all(&skill_b_dir).unwrap();
        fs::write(
            skill_b_dir.join("SKILL.md"),
            r#"---
name: skill-b
description: Skill B depends on D
---
# Skill B
Uses [skill-d](../skill-d/SKILL.md).
"#,
        )
        .unwrap();

        // Create skill C (depends on D)
        let skill_c_dir = skills_dir.join("skill-c");
        fs::create_dir_all(&skill_c_dir).unwrap();
        fs::write(
            skill_c_dir.join("SKILL.md"),
            r#"---
name: skill-c
description: Skill C depends on D
---
# Skill C
Also uses [skill-d](../skill-d/SKILL.md).
"#,
        )
        .unwrap();

        // Create skill D (no dependencies)
        let skill_d_dir = skills_dir.join("skill-d");
        fs::create_dir_all(&skill_d_dir).unwrap();
        fs::write(
            skill_d_dir.join("SKILL.md"),
            r#"---
name: skill-d
description: Skill D has no dependencies
---
# Skill D
Base skill with no dependencies.
"#,
        )
        .unwrap();

        let roots = vec![SkillRoot {
            root: skills_dir.clone(),
            source: skrills_discovery::SkillSource::Extra(0),
        }];

        let service =
            SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

        // Force refresh to build the graph
        service.invalidate_cache().unwrap();
        let skills = service.current_skills_with_dups().unwrap().0;

        // Verify skills were discovered
        assert_eq!(skills.len(), 4);

        // Test dependency resolution
        let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md";
        let deps = service.resolve_dependencies(skill_a_uri).unwrap();

        // Debug output
        eprintln!("skill-a dependencies: {:?}", deps);
        eprintln!(
            "Skills discovered: {:?}",
            skills.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        // skill-a should have transitive dependencies: skill-b, skill-c, skill-d
        assert!(
            deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()),
            "Expected skill-b in deps"
        );
        assert!(
            deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()),
            "Expected skill-c in deps"
        );
        assert!(
            deps.contains(&"skill://skrills/extra0/skill-d/SKILL.md".to_string()),
            "Expected skill-d in deps"
        );

        // Test reverse dependencies
        let skill_d_uri = "skill://skrills/extra0/skill-d/SKILL.md";
        let dependents = service.get_dependents(skill_d_uri).unwrap();

        // skill-d should be used by skill-b and skill-c
        assert!(dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
        assert!(dependents.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));

        // Test transitive dependents
        let trans_deps = service.get_transitive_dependents(skill_d_uri).unwrap();

        // skill-d should transitively affect skill-a, skill-b, skill-c
        assert!(trans_deps.contains(&"skill://skrills/extra0/skill-a/SKILL.md".to_string()));
        assert!(trans_deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
        assert!(trans_deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));
    }

    #[test]
    fn test_resolve_dependencies_tool() {
        use skrills_discovery::SkillRoot;

        // Initialize tracing for test
        let _ = tracing_subscriber::fmt()
            .with_env_filter("skrills::deps=debug")
            .try_init();

        let temp = tempdir().unwrap();
        let skills_dir = temp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create skill A (depends on B)
        let skill_a_dir = skills_dir.join("skill-a");
        fs::create_dir_all(&skill_a_dir).unwrap();
        fs::write(
            skill_a_dir.join("SKILL.md"),
            r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md) for details.
"#,
        )
        .unwrap();

        // Create skill B (depends on C)
        let skill_b_dir = skills_dir.join("skill-b");
        fs::create_dir_all(&skill_b_dir).unwrap();
        fs::write(
            skill_b_dir.join("SKILL.md"),
            r#"---
name: skill-b
description: Skill B depends on C
---
# Skill B
Uses [skill-c](../skill-c/SKILL.md).
"#,
        )
        .unwrap();

        // Create skill C (no dependencies)
        let skill_c_dir = skills_dir.join("skill-c");
        fs::create_dir_all(&skill_c_dir).unwrap();
        fs::write(
            skill_c_dir.join("SKILL.md"),
            r#"---
name: skill-c
description: Skill C has no dependencies
---
# Skill C
Base skill.
"#,
        )
        .unwrap();

        let roots = vec![SkillRoot {
            root: skills_dir.clone(),
            source: skrills_discovery::SkillSource::Extra(0),
        }];

        let service =
            SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

        // Force refresh to build the graph
        service.invalidate_cache().unwrap();

        // Test 1: Transitive dependencies for A (should get B and C)
        let deps = service
            .resolve_dependencies("skill://skrills/extra0/skill-a/SKILL.md")
            .unwrap();
        assert_eq!(deps.len(), 2);
        assert!(deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
        assert!(deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));

        // Test 2: Direct dependencies for A (should only get B)
        let mut cache = service.cache.lock();
        let direct_deps = cache
            .get_direct_dependencies("skill://skrills/extra0/skill-a/SKILL.md")
            .unwrap();
        assert_eq!(direct_deps.len(), 1);
        assert!(direct_deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
        drop(cache);

        // Test 3: Direct dependents of C (should only get B)
        let dependents = service
            .get_dependents("skill://skrills/extra0/skill-c/SKILL.md")
            .unwrap();
        assert_eq!(dependents.len(), 1);
        assert!(dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));

        // Test 4: Transitive dependents of C (should get A and B)
        let trans_dependents = service
            .get_transitive_dependents("skill://skrills/extra0/skill-c/SKILL.md")
            .unwrap();
        assert_eq!(trans_dependents.len(), 2);
        assert!(trans_dependents.contains(&"skill://skrills/extra0/skill-a/SKILL.md".to_string()));
        assert!(trans_dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    }

    #[test]
    fn test_read_resource_without_resolve() {
        use skrills_discovery::SkillRoot;

        let temp = tempdir().unwrap();
        let skills_dir = temp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create skill A (depends on B)
        let skill_a_dir = skills_dir.join("skill-a");
        fs::create_dir_all(&skill_a_dir).unwrap();
        fs::write(
            skill_a_dir.join("SKILL.md"),
            r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md) for details.
"#,
        )
        .unwrap();

        // Create skill B (no dependencies)
        let skill_b_dir = skills_dir.join("skill-b");
        fs::create_dir_all(&skill_b_dir).unwrap();
        fs::write(
            skill_b_dir.join("SKILL.md"),
            r#"---
name: skill-b
description: Skill B
---
# Skill B
Base skill.
"#,
        )
        .unwrap();

        let roots = vec![SkillRoot {
            root: skills_dir.clone(),
            source: skrills_discovery::SkillSource::Extra(0),
        }];

        let service =
            SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

        // Test reading without resolve param
        let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md";
        let result = service.read_resource_sync(skill_a_uri).unwrap();

        // Should return only the requested skill
        assert_eq!(result.contents.len(), 1);
        let content = &result.contents[0];
        if let ResourceContents::TextResourceContents {
            uri, text, meta, ..
        } = content
        {
            assert_eq!(uri, skill_a_uri);
            assert!(text.contains("Skill A"));
            assert!(text.contains("depends on B"));
            // Check metadata indicates this is the requested resource
            let meta = meta.as_ref().unwrap();
            assert_eq!(meta.get("role").and_then(|v| v.as_str()), Some("requested"));
        } else {
            panic!("Expected TextResourceContents");
        }
    }

    #[test]
    fn test_read_resource_with_resolve_true() {
        use skrills_discovery::SkillRoot;

        let temp = tempdir().unwrap();
        let skills_dir = temp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create skill A (depends on B and C)
        let skill_a_dir = skills_dir.join("skill-a");
        fs::create_dir_all(&skill_a_dir).unwrap();
        fs::write(
            skill_a_dir.join("SKILL.md"),
            r#"---
name: skill-a
description: Skill A depends on B and C
---
# Skill A
See [skill-b](../skill-b/SKILL.md) and [skill-c](../skill-c/SKILL.md).
"#,
        )
        .unwrap();

        // Create skill B (depends on D)
        let skill_b_dir = skills_dir.join("skill-b");
        fs::create_dir_all(&skill_b_dir).unwrap();
        fs::write(
            skill_b_dir.join("SKILL.md"),
            r#"---
name: skill-b
description: Skill B depends on D
---
# Skill B
Uses [skill-d](../skill-d/SKILL.md).
"#,
        )
        .unwrap();

        // Create skill C (depends on D)
        let skill_c_dir = skills_dir.join("skill-c");
        fs::create_dir_all(&skill_c_dir).unwrap();
        fs::write(
            skill_c_dir.join("SKILL.md"),
            r#"---
name: skill-c
description: Skill C depends on D
---
# Skill C
Also uses [skill-d](../skill-d/SKILL.md).
"#,
        )
        .unwrap();

        // Create skill D (no dependencies)
        let skill_d_dir = skills_dir.join("skill-d");
        fs::create_dir_all(&skill_d_dir).unwrap();
        fs::write(
            skill_d_dir.join("SKILL.md"),
            r#"---
name: skill-d
description: Skill D
---
# Skill D
Base skill.
"#,
        )
        .unwrap();

        let roots = vec![SkillRoot {
            root: skills_dir.clone(),
            source: skrills_discovery::SkillSource::Extra(0),
        }];

        let service =
            SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

        // Test reading with resolve=true
        let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md?resolve=true";
        let result = service.read_resource_sync(skill_a_uri).unwrap();

        // Should return requested skill + all transitive dependencies (B, C, D)
        assert_eq!(result.contents.len(), 4);

        // First item should be the requested skill
        let first = &result.contents[0];
        if let ResourceContents::TextResourceContents {
            uri, text, meta, ..
        } = first
        {
            assert_eq!(uri, "skill://skrills/extra0/skill-a/SKILL.md");
            assert!(text.contains("Skill A"));
            let meta = meta.as_ref().unwrap();
            assert_eq!(meta.get("role").and_then(|v| v.as_str()), Some("requested"));
        } else {
            panic!("Expected TextResourceContents");
        }

        // Check that dependencies are included
        let uris: Vec<String> = result
            .contents
            .iter()
            .filter_map(|c| match c {
                ResourceContents::TextResourceContents { uri, .. } => Some(uri.clone()),
                _ => None,
            })
            .collect();

        assert!(uris.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
        assert!(uris.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));
        assert!(uris.contains(&"skill://skrills/extra0/skill-d/SKILL.md".to_string()));

        // Check that dependencies have correct role metadata
        for content in &result.contents[1..] {
            if let ResourceContents::TextResourceContents { meta, .. } = content {
                let meta = meta.as_ref().unwrap();
                assert_eq!(
                    meta.get("role").and_then(|v| v.as_str()),
                    Some("dependency")
                );
            }
        }
    }

    #[test]
    fn test_read_resource_with_resolve_false() {
        use skrills_discovery::SkillRoot;

        let temp = tempdir().unwrap();
        let skills_dir = temp.path().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        // Create skill A (depends on B)
        let skill_a_dir = skills_dir.join("skill-a");
        fs::create_dir_all(&skill_a_dir).unwrap();
        fs::write(
            skill_a_dir.join("SKILL.md"),
            r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md).
"#,
        )
        .unwrap();

        // Create skill B
        let skill_b_dir = skills_dir.join("skill-b");
        fs::create_dir_all(&skill_b_dir).unwrap();
        fs::write(
            skill_b_dir.join("SKILL.md"),
            r#"---
name: skill-b
description: Skill B
---
# Skill B
"#,
        )
        .unwrap();

        let roots = vec![SkillRoot {
            root: skills_dir.clone(),
            source: skrills_discovery::SkillSource::Extra(0),
        }];

        let service =
            SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

        // Test reading with resolve=false (explicit)
        let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md?resolve=false";
        let result = service.read_resource_sync(skill_a_uri).unwrap();

        // Should return only the requested skill (same as no param)
        assert_eq!(result.contents.len(), 1);
        let content = &result.contents[0];
        if let ResourceContents::TextResourceContents {
            uri, text, meta, ..
        } = content
        {
            assert_eq!(uri, "skill://skrills/extra0/skill-a/SKILL.md");
            assert!(text.contains("Skill A"));
            let meta = meta.as_ref().unwrap();
            assert_eq!(meta.get("role").and_then(|v| v.as_str()), Some("requested"));
        } else {
            panic!("Expected TextResourceContents");
        }
    }

    #[test]
    fn test_parse_uri_with_query() {
        // Test basic URI without query
        let (base, resolve) = parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md");
        assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
        assert!(!resolve);

        // Test with resolve=true
        let (base, resolve) =
            parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?resolve=true");
        assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
        assert!(resolve);

        // Test with resolve=false
        let (base, resolve) =
            parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?resolve=false");
        assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
        assert!(!resolve);

        // Test with resolve shorthand
        let (base, resolve) =
            parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?resolve");
        assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
        assert!(resolve);

        // Test with multiple params
        let (base, resolve) =
            parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?foo=bar&resolve=true");
        assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
        assert!(resolve);

        // Test with multiple params, resolve first
        let (base, resolve) =
            parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?resolve=true&foo=bar");
        assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
        assert!(resolve);
    }

    #[test]
    fn validate_skills_tool_dependency_validation() {
        let temp = tempdir().unwrap();
        let skill_dir = temp.path().join("skills");
        std::fs::create_dir_all(&skill_dir).unwrap();

        // Create a skill with missing local dependencies
        let skill_path = skill_dir.join("SKILL.md");
        std::fs::write(
            &skill_path,
            r#"---
name: test-skill
description: A test skill with dependencies
---
# Test Skill

This skill references:
- [Missing module](modules/helper.md)
- [Missing reference](references/guide.md)
- [Existing file](../other.md)
"#,
        )
        .unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", temp.path());

        let service =
            SkillService::new_with_ttl(vec![skill_dir.clone()], Duration::from_secs(1)).unwrap();

        // Validate without dependency checking
        let result_no_deps = service
            .validate_skills_tool(
                json!({"target": "both", "check_dependencies": false})
                    .as_object()
                    .cloned()
                    .unwrap(),
            )
            .unwrap();

        let structured_no_deps = result_no_deps.structured_content.unwrap();
        let results_no_deps = structured_no_deps
            .get("results")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(results_no_deps.len(), 1);
        assert!(results_no_deps[0].get("dependency_issues").is_none());

        // Validate with dependency checking
        let result_with_deps = service
            .validate_skills_tool(
                json!({"target": "both", "check_dependencies": true})
                    .as_object()
                    .cloned()
                    .unwrap(),
            )
            .unwrap();

        let structured_with_deps = result_with_deps.structured_content.unwrap();
        let results_with_deps = structured_with_deps
            .get("results")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(results_with_deps.len(), 1);

        let skill_result = &results_with_deps[0];
        let dep_issues = skill_result
            .get("dependency_issues")
            .unwrap()
            .as_array()
            .unwrap();
        let missing_count = skill_result.get("missing_count").unwrap().as_u64().unwrap();

        // Should find missing modules and references
        assert!(
            missing_count >= 2,
            "Expected at least 2 missing dependencies, found {}",
            missing_count
        );

        // Check that dependency issues have the right structure
        let has_missing_module = dep_issues
            .iter()
            .any(|i| i.get("type").unwrap().as_str().unwrap() == "missing_module");
        let has_missing_reference = dep_issues
            .iter()
            .any(|i| i.get("type").unwrap().as_str().unwrap() == "missing_reference");

        assert!(
            has_missing_module,
            "Expected to find missing_module issue type"
        );
        assert!(
            has_missing_reference,
            "Expected to find missing_reference issue type"
        );

        // Verify the summary includes dependency issues
        assert_eq!(
            structured_with_deps.get("check_dependencies").unwrap(),
            &json!(true)
        );
        let total_dep_issues = structured_with_deps
            .get("total_dependency_issues")
            .unwrap()
            .as_u64()
            .unwrap();
        assert!(
            total_dep_issues >= 2,
            "Expected at least 2 total dependency issues"
        );

        match original_home {
            Some(val) => std::env::set_var("HOME", val),
            None => std::env::remove_var("HOME"),
        }
    }

    #[tokio::test]
    async fn sync_all_tool_syncs_skills_into_codex_skills_root() {
        let _guard = crate::test_support::env_guard();
        let temp = tempdir().unwrap();
        let claude_skill = temp.path().join(".claude/skills/example-skill/SKILL.md");
        std::fs::create_dir_all(claude_skill.parent().unwrap()).unwrap();
        std::fs::write(&claude_skill, "example skill").unwrap();

        let original_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", temp.path());

        let service = SkillService::new_with_ttl(vec![], Duration::from_secs(1)).unwrap();
        let _ = service
            .sync_all_tool(
                json!({
                    "from": "claude",
                    "dry_run": false,
                    "skip_existing_commands": true
                })
                .as_object()
                .cloned()
                .unwrap(),
            )
            .unwrap();

        match original_home {
            Some(val) => std::env::set_var("HOME", val),
            None => std::env::remove_var("HOME"),
        }

        // `sync_skills_only_from_claude` preserves paths relative to ~/.claude.
        let expected = temp
            .path()
            .join(".codex/skills/skills/example-skill/SKILL.md");
        assert!(
            expected.exists(),
            "expected skill copied into ~/.codex/skills"
        );

        let unexpected = temp
            .path()
            .join(".codex/skills-mirror/skills/example-skill/SKILL.md");
        assert!(
            !unexpected.exists(),
            "sync-all should not write skills into ~/.codex/skills-mirror"
        );
    }
}
