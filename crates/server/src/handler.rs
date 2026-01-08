//! MCP ServerHandler implementation for SkillService.
//!
//! This module implements the Remote Method Call Protocol (RMCP) `ServerHandler` trait
//! for `SkillService`, providing the core MCP functionality:
//!
//! - `list_resources()` - lists available skill resources
//! - `read_resource()` - reads a specific skill resource by URI
//! - `list_tools()` - lists all MCP tools with their JSON schemas
//! - `call_tool()` - dispatches tool calls to specific handlers
//!
//! # Tool Naming Convention
//!
//! MCP tools accept **both kebab-case and snake_case** for tool names:
//! - `validate-skill` or `validate_skill`
//! - `analyze-tokens` or `analyze_tokens`
//! - `sync-from-claude` or `sync_from_claude`
//!
//! This dual convention exists because MCP clients may normalize tool names differently.
//! Claude Code uses kebab-case internally but MCP spec examples often show snake_case.
//! The canonical names in `list_tools()` use kebab-case, but `call_tool()` normalizes
//! incoming names to support both conventions for compatibility.

use crate::app::SkillService;
use crate::discovery::{is_skill_file, priority_labels_and_rank_map};
use crate::sync::mirror_source_root;
use crate::tool_schemas;
use anyhow::{anyhow, Result};
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, ListResourcesResult, ListToolsResult,
    PaginatedRequestParam, ReadResourceRequestParam, ReadResourceResult,
};
use rmcp::ServerHandler;
use serde_json::json;
use skrills_state::home_dir;
use std::fs;

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
    /// validating skills for CLI compatibility, analyzing token usage,
    /// and synchronizing configurations between Claude Code and Codex CLI.
    ///
    /// Tool schemas are defined in the `tool_schemas` module for maintainability.
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        __context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, rmcp::ErrorData>> + Send + '_
    {
        #[cfg_attr(not(feature = "subagents"), allow(unused_mut))]
        let mut tools = tool_schemas::all_tools();

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
    /// such as validating skills, analyzing token usage, or synchronizing
    /// configurations between Claude Code and Codex CLI.
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
                    "list-subagents"
                        | "list-agents"
                        | "run-subagent"
                        | "run-subagent-async"
                        | "get-run-status"
                        | "get-async-status"
                        | "stop-run"
                        | "get-run-history"
                        | "get-run-events"
                        | "download-transcript-secure"
                        | "list_subagents"
                        | "list_agents"
                        | "run_subagent"
                        | "run_subagent_async"
                        | "get_run_status"
                        | "get_async_status"
                        | "stop_run"
                        | "get_run_history"
                        | "get_run_events"
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
            let result = match request.name.as_ref() {
                "create-skill" => {
                    let args = request.arguments.clone().unwrap_or_default();
                    self.create_skill_tool(args).await
                }
                "search-skills-github" => {
                    let args = request.arguments.clone().unwrap_or_default();
                    self.search_skills_github_tool(args).await
                }
                _ => (|| -> Result<CallToolResult> {
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
                        let codex_skills_root = home.join(".codex/skills");
                        let report = crate::sync::sync_skills_only_from_claude(
                            &claude_root,
                            &codex_skills_root,
                            include_marketplace,
                        )?;
                        let _ = crate::setup::ensure_codex_skills_feature_enabled(
                            &home.join(".codex/config.toml"),
                        );
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
                            let codex_skills_root = home.join(".codex/skills");

                            if dry_run {
                                let count = walkdir::WalkDir::new(&claude_root)
                                    .min_depth(1)
                                    .max_depth(6)
                                    .into_iter()
                                    .filter_map(|e| e.ok())
                                    .filter(is_skill_file)
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
                                let report = crate::sync::sync_skills_only_from_claude(
                                    &claude_root,
                                    &codex_skills_root,
                                    include_marketplace,
                                )?;
                                let _ = crate::setup::ensure_codex_skills_feature_enabled(
                                    &home.join(".codex/config.toml"),
                                );
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
                            use skrills_sync::{
                                ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams,
                            };

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
                            let source = CodexAdapter::new()?;
                            let target = ClaudeAdapter::new()?;
                            let orch = SyncOrchestrator::new(source, target);
                            let report = orch.sync(&params)?;

                            Ok(CallToolResult {
                                content: vec![Content::text(report.summary.clone())],
                                is_error: Some(!report.success),
                                structured_content: Some(json!({
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
                        let args = request.arguments.clone().unwrap_or_default();
                        self.sync_all_tool(args)
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
                        let args = request.arguments.clone().unwrap_or_default();
                        self.validate_skills_tool(args)
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
                                // RATIONALE: json!({...}) with braces always produces Value::Object,
                                // so as_object_mut() cannot fail here.
                                result
                                    .as_object_mut()
                                    .expect("analysis result JSON is an object constructed inline")
                                    .insert(
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
                    "resolve-dependencies" => {
                        let args = request.arguments.clone().unwrap_or_default();

                        // Extract URI
                        let uri = args
                            .get("uri")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("uri parameter is required"))?;

                        // Extract direction (default: dependencies)
                        let direction = args
                            .get("direction")
                            .and_then(|v| v.as_str())
                            .unwrap_or("dependencies");

                        // Extract transitive flag (default: true)
                        let transitive = args
                            .get("transitive")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);

                        // Validate direction
                        if direction != "dependencies" && direction != "dependents" {
                            return Err(anyhow!(
                                "direction must be 'dependencies' or 'dependents'"
                            ));
                        }

                        // Resolve based on direction and transitive flag
                        let results = match (direction, transitive) {
                            ("dependencies", true) => self.resolve_dependencies(uri)?,
                            ("dependencies", false) => {
                                // For non-transitive dependencies, get direct deps only
                                let mut cache = self.cache.lock();
                                cache.get_direct_dependencies(uri)?
                            }
                            ("dependents", true) => self.get_transitive_dependents(uri)?,
                            ("dependents", false) => self.get_dependents(uri)?,
                            _ => unreachable!(),
                        };

                        let text = format!(
                            "Found {} {} for {}",
                            results.len(),
                            if direction == "dependencies" {
                                if transitive {
                                    "transitive dependencies"
                                } else {
                                    "direct dependencies"
                                }
                            } else if transitive {
                                "transitive dependents"
                            } else {
                                "direct dependents"
                            },
                            uri
                        );

                        Ok(CallToolResult {
                            content: vec![Content::text(text)],
                            structured_content: Some(json!({
                                "uri": uri,
                                "direction": direction,
                                "transitive": transitive,
                                "results": results,
                                "count": results.len()
                            })),
                            is_error: Some(false),
                            meta: None,
                        })
                    }
                    "skill-metrics" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        let include_validation = args
                            .get("include_validation")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        let metrics = self.compute_metrics(include_validation)?;

                        let summary = format!(
                            "Metrics for {} skills: {} tokens total, {} high quality",
                            metrics.total_skills,
                            metrics.token_stats.total_tokens,
                            metrics.by_quality.high
                        );

                        Ok(CallToolResult {
                            content: vec![Content::text(summary)],
                            structured_content: Some(serde_json::to_value(&metrics)?),
                            is_error: Some(false),
                            meta: None,
                        })
                    }
                    "recommend-skills" => {
                        let args = request.arguments.clone().unwrap_or_default();

                        let uri = args
                            .get("uri")
                            .and_then(|v| v.as_str())
                            .ok_or_else(|| anyhow!("uri parameter is required"))?;

                        let limit = args
                            .get("limit")
                            .and_then(|v| v.as_u64())
                            .map(|v| v as usize)
                            .unwrap_or(10);

                        let include_quality = args
                            .get("include_quality")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(true);

                        let recommendations =
                            self.recommend_skills(uri, limit, include_quality)?;

                        let summary = format!(
                            "Found {} recommendations for {} ({} dependencies, {} dependents, {} siblings)",
                            recommendations.recommendations.len(),
                            uri,
                            recommendations.recommendations.iter()
                                .filter(|r| matches!(r.relationship, crate::app::RecommendationRelationship::Dependency))
                                .count(),
                            recommendations.recommendations.iter()
                                .filter(|r| matches!(r.relationship, crate::app::RecommendationRelationship::Dependent))
                                .count(),
                            recommendations.recommendations.iter()
                                .filter(|r| matches!(r.relationship, crate::app::RecommendationRelationship::Sibling))
                                .count(),
                        );

                        Ok(CallToolResult {
                            content: vec![Content::text(summary)],
                            structured_content: Some(serde_json::to_value(&recommendations)?),
                            is_error: Some(false),
                            meta: None,
                        })
                    }
                    "skill-loading-status" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        self.skill_loading_status_tool(args)
                    }
                    "enable-skill-trace" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        self.enable_skill_trace_tool(args)
                    }
                    "disable-skill-trace" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        self.disable_skill_trace_tool(args)
                    }
                    "skill-loading-selftest" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        self.skill_loading_selftest_tool(args)
                    }
                    // Intelligence tools (smart recommendations, project context, skill creation)
                    "recommend-skills-smart" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        self.recommend_skills_smart_tool(args)
                    }
                    "analyze-project-context" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        self.analyze_project_context_tool(args)
                    }
                    "suggest-new-skills" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        self.suggest_new_skills_tool(args)
                    }
                    "search-skills-fuzzy" => {
                        let args = request.arguments.clone().unwrap_or_default();
                        self.search_skills_fuzzy_tool(args)
                    }
                    other => Err(anyhow!("unknown tool {other}")),
                }
                })(),
            }
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None));
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::AGENTS_URI;
    use crate::test_support;
    use rmcp::model::{Extensions, Meta, RequestId};
    use rmcp::service::{serve_directly, RequestContext, RunningService};
    use std::future::Future;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn service_with_context(
        service: SkillService,
    ) -> (
        RunningService<rmcp::RoleServer, SkillService>,
        RequestContext<rmcp::RoleServer>,
        tokio::io::DuplexStream,
    ) {
        let (client, server) = tokio::io::duplex(64);
        let running = serve_directly::<rmcp::RoleServer, _, _, _, _>(service, server, None);
        let context = RequestContext {
            ct: CancellationToken::new(),
            id: RequestId::Number(1),
            meta: Meta::new(),
            extensions: Extensions::new(),
            peer: running.peer().clone(),
        };
        (running, context, client)
    }

    fn build_service(temp: &tempfile::TempDir) -> SkillService {
        let skill_root = temp.path().join(".codex/skills/demo");
        std::fs::create_dir_all(&skill_root).expect("create skill root");
        std::fs::write(skill_root.join("SKILL.md"), "demo skill").expect("write skill");
        SkillService::new_with_ttl(Vec::new(), std::time::Duration::from_secs(1))
            .expect("service should build")
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.previous {
                std::env::set_var(self.key, v);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = std::env::var(key).ok();
        if let Some(val) = value {
            std::env::set_var(key, val);
        } else {
            std::env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    fn run_async<T>(future: impl Future<Output = T>) -> T {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime should build");
        let result = runtime.block_on(future);
        runtime.shutdown_timeout(Duration::from_millis(100));
        result
    }

    #[test]
    fn list_resources_includes_agents_doc() {
        /*
        GIVEN a service with a temp home and skill roots
        WHEN listing resources
        THEN the AGENTS.md resource should be included
        */
        let _guard = test_support::env_guard();
        let temp = tempdir().expect("tempdir");
        let _home = set_env_var(
            "HOME",
            Some(
                temp.path()
                    .to_str()
                    .expect("temp home should be valid utf-8"),
            ),
        );

        let service = build_service(&temp);
        let result = run_async(async move {
            let (running, context, _client) = service_with_context(service);
            running
                .service()
                .list_resources(None, context)
                .await
                .expect("list_resources should succeed")
        });

        assert!(
            result.resources.iter().any(|r| r.uri == AGENTS_URI),
            "AGENTS resource should be listed"
        );
    }

    #[test]
    fn list_tools_includes_core_tooling() {
        /*
        GIVEN a service
        WHEN listing tools
        THEN core tools like create-skill should be present
        */
        let _guard = test_support::env_guard();
        let temp = tempdir().expect("tempdir");
        let _home = set_env_var(
            "HOME",
            Some(
                temp.path()
                    .to_str()
                    .expect("temp home should be valid utf-8"),
            ),
        );

        let service = build_service(&temp);
        let result = run_async(async move {
            let (running, context, _client) = service_with_context(service);
            running
                .service()
                .list_tools(None, context)
                .await
                .expect("list_tools should succeed")
        });

        assert!(
            result.tools.iter().any(|tool| tool.name == "create-skill"),
            "create-skill tool should be available"
        );
    }

    #[test]
    fn call_tool_unknown_returns_error() {
        /*
        GIVEN a service
        WHEN calling an unknown tool
        THEN it should return a structured error
        */
        let _guard = test_support::env_guard();
        let temp = tempdir().expect("tempdir");
        let _home = set_env_var(
            "HOME",
            Some(
                temp.path()
                    .to_str()
                    .expect("temp home should be valid utf-8"),
            ),
        );

        let service = build_service(&temp);
        let result = run_async(async move {
            let (running, context, _client) = service_with_context(service);
            running
                .service()
                .call_tool(
                    CallToolRequestParam {
                        name: "does-not-exist".into(),
                        arguments: None,
                    },
                    context,
                )
                .await
        });

        let err = result.expect_err("unknown tool should error");
        assert!(
            err.message.contains("unknown tool"),
            "error message should mention unknown tool"
        );
    }
}
