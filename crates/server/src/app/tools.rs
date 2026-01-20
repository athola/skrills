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

    /// Compares a skill across Claude, Codex, and Copilot to show differences.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `name`: skill name to compare (required)
    /// - `context_lines`: number of context lines for diff (default: 3)
    ///
    /// # Returns
    ///
    /// A `CallToolResult` containing:
    /// - Unified diff output
    /// - Frontmatter comparison
    /// - Token count differences
    pub(crate) fn skill_diff_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        use skrills_discovery::SkillSource;

        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("skill name is required"))?;
        let context_lines = args
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(3) as usize;

        // Get all discovered skills
        let (skills, _) = self.current_skills_with_dups()?;

        // Find matching skills across each source
        let mut claude_skill: Option<(String, String)> = None;
        let mut codex_skill: Option<(String, String)> = None;
        let mut copilot_skill: Option<(String, String)> = None;

        for meta in &skills {
            if meta.name == name {
                let content = fs::read_to_string(&meta.path).ok();
                if let Some(c) = content {
                    let path_str = meta.path.display().to_string();
                    match meta.source {
                        SkillSource::Claude | SkillSource::Marketplace | SkillSource::Cache => {
                            if claude_skill.is_none() {
                                claude_skill = Some((path_str, c));
                            }
                        }
                        SkillSource::Codex | SkillSource::Mirror => {
                            if codex_skill.is_none() {
                                codex_skill = Some((path_str, c));
                            }
                        }
                        SkillSource::Copilot => {
                            if copilot_skill.is_none() {
                                copilot_skill = Some((path_str, c));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        // Check if skill exists anywhere
        if claude_skill.is_none() && codex_skill.is_none() && copilot_skill.is_none() {
            return Ok(CallToolResult {
                content: vec![Content::text(format!(
                    "Skill '{}' not found in any location",
                    name
                ))],
                is_error: Some(true),
                structured_content: Some(json!({
                    "error": "skill_not_found",
                    "name": name
                })),
                meta: None,
            });
        }

        // Generate diffs
        let mut diffs = Vec::new();
        let mut locations = Vec::new();

        if let Some((path, _)) = &claude_skill {
            locations.push(json!({"source": "claude", "path": path}));
        }
        if let Some((path, _)) = &codex_skill {
            locations.push(json!({"source": "codex", "path": path}));
        }
        if let Some((path, _)) = &copilot_skill {
            locations.push(json!({"source": "copilot", "path": path}));
        }

        // Helper to generate unified diff
        fn unified_diff(a: &str, b: &str, label_a: &str, label_b: &str, context: usize) -> String {
            let a_lines: Vec<&str> = a.lines().collect();
            let b_lines: Vec<&str> = b.lines().collect();

            if a_lines == b_lines {
                return String::new();
            }

            let mut output = String::new();
            output.push_str(&format!("--- {}\n", label_a));
            output.push_str(&format!("+++ {}\n", label_b));

            // Simple line-by-line diff with context
            let max_len = a_lines.len().max(b_lines.len());
            let mut in_hunk = false;
            let mut hunk_start = 0;

            for i in 0..max_len {
                let a_line = a_lines.get(i).copied();
                let b_line = b_lines.get(i).copied();

                if a_line != b_line {
                    if !in_hunk {
                        // Start new hunk with context
                        hunk_start = i.saturating_sub(context);
                        output.push_str(&format!(
                            "@@ -{},{} +{},{} @@\n",
                            hunk_start + 1,
                            context * 2 + 1,
                            hunk_start + 1,
                            context * 2 + 1
                        ));

                        // Add leading context
                        for j in hunk_start..i {
                            if let Some(line) = a_lines.get(j) {
                                output.push_str(&format!(" {}\n", line));
                            }
                        }
                        in_hunk = true;
                    }

                    if let Some(line) = a_line {
                        output.push_str(&format!("-{}\n", line));
                    }
                    if let Some(line) = b_line {
                        output.push_str(&format!("+{}\n", line));
                    }
                } else if in_hunk {
                    // Trailing context
                    if let Some(line) = a_line {
                        output.push_str(&format!(" {}\n", line));
                    }
                    if i >= hunk_start + context * 2 {
                        in_hunk = false;
                    }
                }
            }

            output
        }

        // Count tokens (simple word count approximation)
        fn estimate_tokens(content: &str) -> usize {
            content.split_whitespace().count()
        }

        let mut token_counts = json!({});

        // Compare Claude vs Codex
        if let (Some((_, claude_content)), Some((_, codex_content))) = (&claude_skill, &codex_skill)
        {
            let diff = unified_diff(
                claude_content,
                codex_content,
                "claude",
                "codex",
                context_lines,
            );
            if !diff.is_empty() {
                diffs.push(json!({
                    "comparison": "claude_vs_codex",
                    "diff": diff,
                    "identical": false
                }));
            } else {
                diffs.push(json!({
                    "comparison": "claude_vs_codex",
                    "identical": true
                }));
            }
        }

        // Compare Claude vs Copilot
        if let (Some((_, claude_content)), Some((_, copilot_content))) =
            (&claude_skill, &copilot_skill)
        {
            let diff = unified_diff(
                claude_content,
                copilot_content,
                "claude",
                "copilot",
                context_lines,
            );
            if !diff.is_empty() {
                diffs.push(json!({
                    "comparison": "claude_vs_copilot",
                    "diff": diff,
                    "identical": false
                }));
            } else {
                diffs.push(json!({
                    "comparison": "claude_vs_copilot",
                    "identical": true
                }));
            }
        }

        // Compare Codex vs Copilot
        if let (Some((_, codex_content)), Some((_, copilot_content))) =
            (&codex_skill, &copilot_skill)
        {
            let diff = unified_diff(
                codex_content,
                copilot_content,
                "codex",
                "copilot",
                context_lines,
            );
            if !diff.is_empty() {
                diffs.push(json!({
                    "comparison": "codex_vs_copilot",
                    "diff": diff,
                    "identical": false
                }));
            } else {
                diffs.push(json!({
                    "comparison": "codex_vs_copilot",
                    "identical": true
                }));
            }
        }

        // Calculate token counts
        if let Some((_, content)) = &claude_skill {
            token_counts["claude"] = json!(estimate_tokens(content));
        }
        if let Some((_, content)) = &codex_skill {
            token_counts["codex"] = json!(estimate_tokens(content));
        }
        if let Some((_, content)) = &copilot_skill {
            token_counts["copilot"] = json!(estimate_tokens(content));
        }

        // Generate summary text
        let identical_count = diffs
            .iter()
            .filter(|d| d["identical"].as_bool().unwrap_or(false))
            .count();
        let diff_count = diffs.len() - identical_count;

        let mut summary = format!("Skill '{}' comparison:\n", name);
        summary.push_str(&format!("  Found in {} locations\n", locations.len()));
        if diff_count > 0 {
            summary.push_str(&format!("  {} differences found\n", diff_count));
        } else if !diffs.is_empty() {
            summary.push_str("  All versions are identical\n");
        }

        // Add token summary
        summary.push_str("\nToken counts:\n");
        if let Some(c) = token_counts.get("claude").and_then(|v| v.as_u64()) {
            summary.push_str(&format!("  Claude: ~{} tokens\n", c));
        }
        if let Some(c) = token_counts.get("codex").and_then(|v| v.as_u64()) {
            summary.push_str(&format!("  Codex: ~{} tokens\n", c));
        }
        if let Some(c) = token_counts.get("copilot").and_then(|v| v.as_u64()) {
            summary.push_str(&format!("  Copilot: ~{} tokens\n", c));
        }

        // Add diff output
        for diff_item in &diffs {
            if let Some(diff_text) = diff_item.get("diff").and_then(|v| v.as_str()) {
                if !diff_text.is_empty() {
                    summary.push_str(&format!("\n{}\n", diff_text));
                }
            }
        }

        Ok(CallToolResult {
            content: vec![Content::text(summary)],
            is_error: Some(false),
            structured_content: Some(json!({
                "name": name,
                "locations": locations,
                "comparisons": diffs,
                "token_counts": token_counts,
                "context_lines": context_lines
            })),
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

    // ─────────────────────────────────────────────────────────────────────────
    // Copilot sync tools
    // ─────────────────────────────────────────────────────────────────────────

    /// Sync from GitHub Copilot CLI to Claude or Codex.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `to`: `"claude"` or `"codex"` (default: `"claude"`) - target CLI
    /// - `dry_run`: `bool` (default: `false`) - preview changes without writing
    ///
    /// # Returns
    ///
    /// A `CallToolResult` with sync summary and structured report.
    pub(crate) fn sync_from_copilot_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_sync::{ClaudeAdapter, CopilotAdapter, SyncOrchestrator, SyncParams};

        let to = args.get("to").and_then(|v| v.as_str()).unwrap_or("claude");
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let params = SyncParams {
            from: Some("copilot".to_string()),
            dry_run,
            sync_commands: true,
            sync_mcp_servers: true,
            sync_preferences: true,
            sync_skills: true,
            ..Default::default()
        };

        let source = CopilotAdapter::new()?;
        let report = if to == "codex" {
            use skrills_sync::CodexAdapter;
            let target = CodexAdapter::new()?;
            SyncOrchestrator::new(source, target).sync(&params)?
        } else {
            let target = ClaudeAdapter::new()?;
            SyncOrchestrator::new(source, target).sync(&params)?
        };

        Ok(CallToolResult {
            content: vec![Content::text(report.summary.clone())],
            is_error: Some(!report.success),
            structured_content: Some(json!({
                "from": "copilot",
                "to": to,
                "dry_run": dry_run,
                "report": report
            })),
            meta: None,
        })
    }

    /// Sync to GitHub Copilot CLI from Claude or Codex.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `from`: `"claude"` or `"codex"` (default: `"claude"`) - source CLI
    /// - `dry_run`: `bool` (default: `false`) - preview changes without writing
    ///
    /// # Returns
    ///
    /// A `CallToolResult` with sync summary and structured report.
    pub(crate) fn sync_to_copilot_tool(
        &self,
        args: JsonMap<String, Value>,
    ) -> Result<CallToolResult> {
        use skrills_sync::{ClaudeAdapter, CopilotAdapter, SyncOrchestrator, SyncParams};

        let from = args
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("claude");
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let params = SyncParams {
            from: Some(from.to_string()),
            dry_run,
            sync_commands: true,
            sync_mcp_servers: true,
            sync_preferences: true,
            sync_skills: true,
            ..Default::default()
        };

        let target = CopilotAdapter::new()?;
        let report = if from == "codex" {
            use skrills_sync::CodexAdapter;
            let source = CodexAdapter::new()?;
            SyncOrchestrator::new(source, target).sync(&params)?
        } else {
            let source = ClaudeAdapter::new()?;
            SyncOrchestrator::new(source, target).sync(&params)?
        };

        Ok(CallToolResult {
            content: vec![Content::text(report.summary.clone())],
            is_error: Some(!report.success),
            structured_content: Some(json!({
                "from": from,
                "to": "copilot",
                "dry_run": dry_run,
                "report": report
            })),
            meta: None,
        })
    }

    /// Syncs skills between Claude, Codex, and Copilot.
    ///
    /// # Arguments
    ///
    /// The `args` map accepts the following JSON keys:
    /// - `from`: `"claude"`, `"codex"`, or `"copilot"` (default: `"claude"`) - source CLI
    /// - `to`: `"claude"`, `"codex"`, or `"copilot"` (default based on from) - target CLI
    /// - `dry_run`: `bool` (default: `false`) - preview changes without writing
    /// - `include_marketplace`: `bool` (default: `false`) - include marketplace skills
    ///
    /// # Returns
    ///
    /// A `CallToolResult` with sync summary and structured report.
    pub(crate) fn sync_skills_tool(&self, args: JsonMap<String, Value>) -> Result<CallToolResult> {
        use skrills_sync::{
            ClaudeAdapter, CodexAdapter, CopilotAdapter, SyncOrchestrator, SyncParams,
        };

        let from = args
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("claude");
        // Default target: codex for claude source, claude for others
        let default_to = if from == "claude" { "codex" } else { "claude" };
        let to = args
            .get("to")
            .and_then(|v| v.as_str())
            .unwrap_or(default_to);
        let dry_run = args
            .get("dry_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let include_marketplace = args
            .get("include_marketplace")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Prevent syncing to self
        if from == to {
            return Err(anyhow::anyhow!(
                "Source and target cannot be the same: {}",
                from
            ));
        }

        let params = SyncParams {
            from: Some(from.to_string()),
            dry_run,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: true,
            include_marketplace,
            ..Default::default()
        };

        // Create adapters and run sync based on from/to combination
        let report = match (from, to) {
            ("claude", "codex") => {
                let source = ClaudeAdapter::new()?;
                let target = CodexAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            }
            ("claude", "copilot") => {
                let source = ClaudeAdapter::new()?;
                let target = CopilotAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            }
            ("codex", "claude") => {
                let source = CodexAdapter::new()?;
                let target = ClaudeAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            }
            ("codex", "copilot") => {
                let source = CodexAdapter::new()?;
                let target = CopilotAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            }
            ("copilot", "claude") => {
                let source = CopilotAdapter::new()?;
                let target = ClaudeAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            }
            ("copilot", "codex") => {
                let source = CopilotAdapter::new()?;
                let target = CodexAdapter::new()?;
                SyncOrchestrator::new(source, target).sync(&params)?
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Invalid sync direction: {} -> {}. Valid options are: claude, codex, copilot",
                    from,
                    to
                ));
            }
        };

        Ok(CallToolResult {
            content: vec![Content::text(report.summary.clone())],
            is_error: Some(!report.success),
            structured_content: Some(json!({
                "from": from,
                "to": to,
                "dry_run": dry_run,
                "include_marketplace": include_marketplace,
                "summary": report.summary,
                "skills": {
                    "written": report.skills.written,
                    "skipped": report.skills.skipped.len(),
                }
            })),
            meta: None,
        })
    }
}
