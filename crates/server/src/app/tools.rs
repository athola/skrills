//! Tool handler implementations for `SkillService`.
//!
//! This module contains the MCP tool handlers that are exposed as callable tools
//! to the LLM. Each handler processes arguments and returns a `CallToolResult`.

use super::SkillService;
use crate::skill_trace::{self, ClientTarget as TraceTarget, TraceInstallOptions};
use anyhow::Result;
use rmcp::model::{CallToolResult, Content};
use serde_json::{json, Map as JsonMap, Value};
use skrills_state::home_dir;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

impl SkillService {
    /// Parse the trace target from tool arguments.
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

    /// Validates all discovered skills and optionally autofixes issues.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `target`: `"claude"`, `"codex"`, or `"both"` (default: `"both"`) - which clients to validate for
    /// - `errors_only`: `bool` (default: `false`) - only return skills with errors
    /// - `autofix`: `bool` (default: `false`) - attempt to autofix frontmatter issues
    /// - `check_dependencies`: `bool` (default: `false`) - validate skill dependencies exist
    ///
    /// # Returns
    ///
    /// A `CallToolResult` containing:
    /// - Summary text with validation counts
    /// - Structured JSON array of validation results per skill
    pub(crate) fn validate_skills_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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
                Err(e) => {
                    tracing::warn!(path = %meta.path.display(), error = %e, "Failed to read skill file during validation, skipping");
                    continue;
                }
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
                match autofix_frontmatter(&meta.path, &content, &opts) {
                    Ok(fix_result) => {
                        if fix_result.modified {
                            autofixed += 1;
                            autofixed_skill = true;
                            content = fs::read_to_string(&meta.path).unwrap_or(content);
                            result = validate_skill(&meta.path, &content, validation_target);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(path = %meta.path.display(), error = %e, "Autofix failed for skill");
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
                match self.resolve_dependencies(&skill_uri) {
                    Ok(deps) => {
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
                    Err(e) => {
                        tracing::warn!(skill_uri = %skill_uri, error = %e, "Failed to resolve skill dependencies");
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
                    // Defensive pattern: while json!({}) always produces Value::Object,
                    // using match with logging handles invariant violations gracefully.
                    match skill_json.as_object_mut() {
                        Some(skill_object) => {
                            skill_object
                                .insert("dependency_issues".to_string(), json!(dependency_issues));
                            skill_object
                                .insert("dependency_count".to_string(), json!(dependency_count));
                            skill_object.insert("missing_count".to_string(), json!(missing_count));
                        }
                        None => {
                            tracing::error!(
                                skill_name = %meta.name,
                                "INVARIANT VIOLATION: skill_json not an object"
                            );
                        }
                    }
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
            if let Some(obj) = structured.as_object_mut() {
                obj.insert("check_dependencies".to_string(), json!(true));
                obj.insert(
                    "total_dependency_issues".to_string(),
                    json!(total_dep_issues),
                );
            }
        }

        Ok(CallToolResult {
            content: vec![Content::text(text)],
            structured_content: Some(structured),
            is_error: Some(false),
            meta: None,
        })
    }

    /// Syncs configuration (commands, skills, MCP servers, preferences) between Claude and Codex.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `from`: `"claude"` or `"codex"` (default: `"claude"`) - source of truth
    /// - `dry_run`: `bool` (default: `false`) - preview changes without writing
    /// - `include_marketplace`: `bool` (default: `false`) - include marketplace skills in sync
    /// - `skip_existing_commands`: `bool` (default: `false`) - skip commands that already exist at destination
    ///
    /// # Returns
    ///
    /// A `CallToolResult` with sync summary text and structured report including
    /// commands, skills, MCP servers, and preferences copied.
    pub(crate) fn sync_all_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        use crate::sync::mirror_source_root;
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

    /// Gets skill loading status for observability and debugging.
    ///
    /// Reports on discovered skill files, instrumentation markers, and trace installation state.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `target`: `"claude"`, `"codex"`, or `"both"` (default: `"both"`)
    /// - `include_cache`: `bool` (default: `false`) - include cached skills in status
    /// - `include_marketplace`: `bool` (default: `false`) - include marketplace skills
    /// - `include_mirror`: `bool` (default: `true`) - include mirrored skills
    /// - `include_agent`: `bool` (default: `true`) - include agent-specific skills
    ///
    /// # Returns
    ///
    /// A `CallToolResult` with skill file counts and structured status JSON.
    pub(crate) fn skill_loading_status_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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

    /// Enables skill tracing for debugging and observability.
    ///
    /// Installs trace skills and instruments existing skills to emit loading events.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `target`: `"claude"`, `"codex"`, or `"both"` (default: `"both"`)
    /// - `instrument`: `bool` (default: `true`) - add trace markers to skills
    /// - `backup`: `bool` (default: `true`) - backup files before modification
    /// - `dry_run`: `bool` (default: `false`) - preview changes without writing
    /// - `include_cache`, `include_marketplace`, `include_mirror`, `include_agent`: `bool` - scope filters
    ///
    /// # Returns
    ///
    /// A `CallToolResult` with trace installation summary and structured report.
    pub(crate) fn enable_skill_trace_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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

    /// Disables skill tracing by removing trace skills and instrumentation.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `target`: `"claude"`, `"codex"`, or `"both"` (default: `"both"`)
    /// - `dry_run`: `bool` (default: `false`) - preview removals without deleting
    ///
    /// # Returns
    ///
    /// A `CallToolResult` with removal summary and list of removed directories.
    pub(crate) fn disable_skill_trace_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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

    /// Runs a self-test to verify skill loading is working correctly.
    ///
    /// Installs a probe skill that responds to a unique token, allowing verification
    /// that skills are being loaded and processed by the agent.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `target`: `"claude"`, `"codex"`, or `"both"` (default: `"both"`)
    /// - `dry_run`: `bool` (default: `false`) - preview without installing probe
    ///
    /// # Returns
    ///
    /// A `CallToolResult` with probe details: the line to send and expected response.
    pub(crate) fn skill_loading_selftest_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
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
