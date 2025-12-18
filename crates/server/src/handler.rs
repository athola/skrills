//! MCP ServerHandler implementation for SkillService.
//!
//! This module implements the Remote Method Call Protocol (RMCP) `ServerHandler` trait
//! for `SkillService`, providing the core MCP functionality:
//!
//! - `list_resources()` - lists available skill resources
//! - `read_resource()` - reads a specific skill resource by URI
//! - `list_tools()` - lists all MCP tools with their JSON schemas
//! - `call_tool()` - dispatches tool calls to specific handlers

use crate::app::SkillService;
use crate::discovery::{is_skill_file, priority_labels_and_rank_map};
use crate::sync::mirror_source_root;
use anyhow::{anyhow, Result};
use rmcp::model::{
    CallToolRequestParam, CallToolResult, Content, ListResourcesResult, ListToolsResult,
    PaginatedRequestParam, ReadResourceRequestParam, ReadResourceResult, Tool, ToolAnnotations,
};
use rmcp::ServerHandler;
use serde_json::{json, Map as JsonMap};
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
                    "Copy SKILL.md files from ~/.claude into ~/.codex/skills (Codex discovery root)".into(),
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
                             "autofix": {
                                 "type": "boolean",
                                 "default": false,
                                 "description": "Automatically fix validation issues when possible"
                             },
                             "errors_only": {
                                 "type": "boolean",
                                 "default": false,
                                 "description": "Only return skills with errors"
                             },
                             "check_dependencies": {
                                 "type": "boolean",
                                 "default": false,
                                 "description": "Validate that skill dependencies exist and are resolvable"
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
            Tool {
                name: "resolve-dependencies".into(),
                title: Some("Resolve skill dependencies".into()),
                description: Some(
                    "Get transitive dependencies or dependents for a skill.".into(),
                ),
                input_schema: std::sync::Arc::new({
                    let mut schema = JsonMap::new();
                    schema.insert("type".into(), json!("object"));
                    schema.insert(
                        "properties".into(),
                        json!({
                            "uri": {
                                "type": "string",
                                "description": "Skill URI (e.g., skill://skrills/user/my-skill)"
                            },
                            "direction": {
                                "type": "string",
                                "enum": ["dependencies", "dependents"],
                                "default": "dependencies",
                                "description": "Direction to traverse: dependencies (what this skill needs) or dependents (what uses this skill)"
                            },
                            "transitive": {
                                "type": "boolean",
                                "default": true,
                                "description": "Include transitive relationships"
                            }
                        }),
                    );
                    schema.insert("required".into(), json!(["uri"]));
                    schema
                }),
                output_schema: None,
                annotations: Some(ToolAnnotations::default()),
                icons: None,
                meta: None,
            },
            Tool {
                name: "skill-metrics".into(),
                title: Some("Get skill statistics and metrics".into()),
                description: Some(
                    "Returns aggregate statistics about discovered skills including counts, quality distribution, dependency patterns, and token usage.".into(),
                ),
                input_schema: std::sync::Arc::new({
                    let mut schema = JsonMap::new();
                    schema.insert("type".into(), json!("object"));
                    schema.insert(
                        "properties".into(),
                        json!({
                            "include_validation": {
                                "type": "boolean",
                                "default": false,
                                "description": "Include validation summary (slower)"
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
                name: "skill-loading-status".into(),
                title: Some("Skill loading status (filesystem + instrumentation)".into()),
                description: Some(
                    "Checks skill roots on disk and reports whether trace/probe skills are installed and whether skill files are instrumented with skrills markers.".into(),
                ),
                input_schema: std::sync::Arc::new({
                    let mut schema = JsonMap::new();
                    schema.insert("type".into(), json!("object"));
                    schema.insert(
                        "properties".into(),
                        json!({
                            "target": { "type": "string", "description": "Target client: claude, codex, or both", "default": "both" },
                            "include_mirror": { "type": "boolean", "default": true, "description": "Include ~/.codex/skills-mirror when target includes codex" },
                            "include_agent": { "type": "boolean", "default": true, "description": "Include ~/.agent/skills" },
                            "include_cache": { "type": "boolean", "default": false, "description": "Include ~/.claude/plugins/cache when target includes claude" },
                            "include_marketplace": { "type": "boolean", "default": false, "description": "Include ~/.claude/plugins/marketplaces when target includes claude" }
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
                name: "enable-skill-trace".into(),
                title: Some("Enable deterministic skill tracing".into()),
                description: Some(
                    "Installs skrills trace/probe skills and (optionally) instruments SKILL.md files with markers so the trace skill can report which skills were loaded.".into(),
                ),
                input_schema: std::sync::Arc::new({
                    let mut schema = JsonMap::new();
                    schema.insert("type".into(), json!("object"));
                    schema.insert(
                        "properties".into(),
                        json!({
                            "target": { "type": "string", "description": "Target client: claude, codex, or both", "default": "both" },
                            "instrument": { "type": "boolean", "default": true, "description": "Append skrills markers to SKILL.md files under selected roots" },
                            "backup": { "type": "boolean", "default": true, "description": "Create .md.bak backups before modifying SKILL.md files" },
                            "dry_run": { "type": "boolean", "default": false, "description": "Preview without writing files" },
                            "include_mirror": { "type": "boolean", "default": true, "description": "Include ~/.codex/skills-mirror when instrumenting codex" },
                            "include_agent": { "type": "boolean", "default": true, "description": "Include ~/.agent/skills when instrumenting" },
                            "include_cache": { "type": "boolean", "default": false, "description": "Include ~/.claude/plugins/cache when instrumenting claude" },
                            "include_marketplace": { "type": "boolean", "default": false, "description": "Include ~/.claude/plugins/marketplaces when instrumenting claude" }
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
                name: "disable-skill-trace".into(),
                title: Some("Disable skill tracing".into()),
                description: Some(
                    "Removes the skrills trace/probe skill directories from primary Claude/Codex skill roots (does not remove instrumentation markers).".into(),
                ),
                input_schema: std::sync::Arc::new({
                    let mut schema = JsonMap::new();
                    schema.insert("type".into(), json!("object"));
                    schema.insert(
                        "properties".into(),
                        json!({
                            "target": { "type": "string", "description": "Target client: claude, codex, or both", "default": "both" },
                            "dry_run": { "type": "boolean", "default": false, "description": "Preview without deleting directories" }
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
                name: "skill-loading-selftest".into(),
                title: Some("Skill loading selftest (probe)".into()),
                description: Some(
                    "Ensures the probe skill exists and returns a one-shot probe line + expected response to confirm skills are loading in the current session.".into(),
                ),
                input_schema: std::sync::Arc::new({
                    let mut schema = JsonMap::new();
                    schema.insert("type".into(), json!("object"));
                    schema.insert(
                        "properties".into(),
                        json!({
                            "target": { "type": "string", "description": "Target client: claude, codex, or both", "default": "both" },
                            "dry_run": { "type": "boolean", "default": false, "description": "Preview without writing probe skill" }
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
                        | "run-subagent"
                        | "run-subagent-async"
                        | "get-run-status"
                        | "get-async-status"
                        | "stop-run"
                        | "get-run-history"
                        | "download-transcript-secure"
                        | "list_subagents"
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
                    other => Err(anyhow!("unknown tool {other}")),
                }
            }()
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None));
            result
        })
    }
}
