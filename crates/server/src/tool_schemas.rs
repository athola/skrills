//! MCP tool schema definitions for SkillService.
//!
//! This module contains the JSON schema definitions for all MCP tools,
//! organized into logical groups:
//!
//! - Sync tools: cross-agent synchronization (sync-skills, sync-commands, etc.)
//! - Validation tools: skill validation and analysis
//! - Dependency tools: dependency resolution and tracking
//! - Metrics tools: skill statistics and metrics
//! - Trace tools: skill loading instrumentation

use rmcp::model::{Tool, ToolAnnotations};
use serde_json::{json, Map as JsonMap};
use std::sync::Arc;

/// Returns an empty object schema for parameterless tools.
///
/// Codex CLI expects every tool input_schema to include a JSON Schema "type".
/// An empty map triggers "missing field `type`" during MCP -> OpenAI conversion,
/// so explicitly mark parameterless tools as taking an empty object.
pub(crate) fn empty_schema() -> Arc<JsonMap<String, serde_json::Value>> {
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), json!({}));
    schema.insert("additionalProperties".into(), json!(false));
    Arc::new(schema)
}

/// Returns the schema for sync tools (from, dry_run, force parameters).
fn sync_schema() -> Arc<JsonMap<String, serde_json::Value>> {
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("object"));
    schema.insert(
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
    schema.insert("additionalProperties".into(), json!(false));
    Arc::new(schema)
}

/// Returns sync-related tools.
///
/// Tools: sync-from-claude, sync-skills, sync-commands, sync-mcp-servers,
/// sync-preferences, sync-all, sync-status
pub(crate) fn sync_tools() -> Vec<Tool> {
    let schema_empty = empty_schema();
    let sync_schema = sync_schema();

    vec![
        Tool {
            name: "sync-from-claude".into(),
            title: Some("Copy ~/.claude skills into ~/.codex".into()),
            description: Some(
                "Copy SKILL.md files from ~/.claude into ~/.codex/skills (Codex discovery root)"
                    .into(),
            ),
            input_schema: schema_empty,
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
        Tool {
            name: "sync-skills".into(),
            title: Some("Sync skills between agents".into()),
            description: Some(
                "Sync SKILL.md files between Claude and Codex. Use --from to specify source."
                    .into(),
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
            description: Some("Sync slash command definitions between Claude and Codex.".into()),
            input_schema: sync_schema.clone(),
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
        Tool {
            name: "sync-mcp-servers".into(),
            title: Some("Sync MCP server configurations".into()),
            description: Some("Sync MCP server configurations between Claude and Codex.".into()),
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
            description: Some("Show what would be synced without making changes (dry run).".into()),
            input_schema: sync_schema,
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
    ]
}

/// Returns validation and analysis tools.
///
/// Tools: validate-skills, analyze-skills
pub(crate) fn validation_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "validate-skills".into(),
            title: Some("Validate skills for CLI compatibility".into()),
            description: Some(
                "Validate skills for Claude Code and/or Codex CLI compatibility. Returns validation errors and warnings.".into(),
            ),
            input_schema: Arc::new({
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
            input_schema: Arc::new({
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
    ]
}

/// Returns dependency resolution tools.
///
/// Tools: resolve-dependencies
pub(crate) fn dependency_tools() -> Vec<Tool> {
    vec![Tool {
        name: "resolve-dependencies".into(),
        title: Some("Resolve skill dependencies".into()),
        description: Some("Get transitive dependencies or dependents for a skill.".into()),
        input_schema: Arc::new({
            let mut schema = JsonMap::new();
            schema.insert("type".into(), json!("object"));
            schema.insert(
                "properties".into(),
                json!({
                    "uri": {
                        "type": "string",
                        "description": "Skill URI (e.g., skill://skrills/codex/my-skill/SKILL.md)"
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
    }]
}

/// Returns recommendation tools.
///
/// Tools: recommend-skills
pub(crate) fn recommend_tools() -> Vec<Tool> {
    vec![Tool {
        name: "recommend-skills".into(),
        title: Some("Get skill recommendations".into()),
        description: Some(
            "Recommends related skills based on dependency relationships. Given a skill URI, suggests dependencies, dependents, and sibling skills (those sharing common dependencies).".into(),
        ),
        input_schema: Arc::new({
            let mut schema = JsonMap::new();
            schema.insert("type".into(), json!("object"));
            schema.insert(
                "properties".into(),
                json!({
                    "uri": {
                        "type": "string",
                        "description": "Skill URI to get recommendations for (e.g., skill://skrills/codex/my-skill/SKILL.md)"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 10,
                        "description": "Maximum number of recommendations to return"
                    },
                    "include_quality": {
                        "type": "boolean",
                        "default": true,
                        "description": "Include quality scores in recommendations"
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
    }]
}

/// Returns metrics tools.
///
/// Tools: skill-metrics
pub(crate) fn metrics_tools() -> Vec<Tool> {
    vec![Tool {
        name: "skill-metrics".into(),
        title: Some("Get skill statistics and metrics".into()),
        description: Some(
            "Returns aggregate statistics about discovered skills including counts, quality distribution, dependency patterns, and token usage.".into(),
        ),
        input_schema: Arc::new({
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
    }]
}

/// Returns skill trace and instrumentation tools.
///
/// Tools: skill-loading-status, enable-skill-trace, disable-skill-trace, skill-loading-selftest
pub(crate) fn trace_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "skill-loading-status".into(),
            title: Some("Skill loading status (filesystem + instrumentation)".into()),
            description: Some(
                "Checks skill roots on disk and reports whether trace/probe skills are installed and whether skill files are instrumented with skrills markers.".into(),
            ),
            input_schema: Arc::new({
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
            input_schema: Arc::new({
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
            input_schema: Arc::new({
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
            input_schema: Arc::new({
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
    ]
}

/// Returns intelligent recommendation and skill creation tools.
///
/// Tools: recommend-skills-smart, analyze-project-context, suggest-new-skills,
/// create-skill, search-skills-github
pub(crate) fn intelligence_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "recommend-skills-smart".into(),
            title: Some("Smart skill recommendations".into()),
            description: Some(
                "Enhanced recommendations combining dependency relationships, usage patterns, \
                 and project context. Returns scored recommendations with explanations."
                    .into(),
            ),
            input_schema: Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "uri": {
                            "type": "string",
                            "description": "Optional skill URI for relationship-based recommendations"
                        },
                        "prompt": {
                            "type": "string",
                            "description": "Optional prompt text for semantic matching"
                        },
                        "project_dir": {
                            "type": "string",
                            "description": "Project directory for context analysis (defaults to cwd)"
                        },
                        "limit": {
                            "type": "integer",
                            "default": 10,
                            "description": "Maximum recommendations to return"
                        },
                        "include_usage": {
                            "type": "boolean",
                            "default": true,
                            "description": "Include usage pattern analysis"
                        },
                        "include_context": {
                            "type": "boolean",
                            "default": true,
                            "description": "Include project context analysis"
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
            name: "analyze-project-context".into(),
            title: Some("Analyze project context".into()),
            description: Some(
                "Analyzes the current project to build a context profile including \
                 languages, dependencies, frameworks, and keywords."
                    .into(),
            ),
            input_schema: Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "project_dir": {
                            "type": "string",
                            "description": "Project directory to analyze (defaults to cwd)"
                        },
                        "include_git": {
                            "type": "boolean",
                            "default": true,
                            "description": "Include git commit keyword analysis"
                        },
                        "commit_limit": {
                            "type": "integer",
                            "default": 50,
                            "description": "Number of recent commits to analyze"
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
            name: "suggest-new-skills".into(),
            title: Some("Suggest skills to create".into()),
            description: Some(
                "Identifies gaps in your skill library based on project context \
                 and usage patterns, suggesting new skills to create."
                    .into(),
            ),
            input_schema: Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "project_dir": {
                            "type": "string",
                            "description": "Project directory for context"
                        },
                        "focus_areas": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Specific areas to focus on (e.g., 'testing', 'deployment')"
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
            name: "create-skill".into(),
            title: Some("Create a new skill".into()),
            description: Some(
                "Creates a new skill via GitHub search, LLM generation, or both. \
                 Default behavior: search GitHub first, then generate if not found."
                    .into(),
            ),
            input_schema: Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "name": {
                            "type": "string",
                            "description": "Name or topic for the skill"
                        },
                        "description": {
                            "type": "string",
                            "description": "Detailed description of what the skill should do"
                        },
                        "method": {
                            "type": "string",
                            "enum": ["github", "llm", "both"],
                            "default": "both",
                            "description": "Creation method: 'github' (search), 'llm' (generate), or 'both'"
                        },
                        "target_dir": {
                            "type": "string",
                            "description": "Directory to create skill in (defaults to installed client, Claude preferred)"
                        },
                        "dry_run": {
                            "type": "boolean",
                            "default": false,
                            "description": "Preview without creating files"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["name", "description"]));
                schema
            }),
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
        Tool {
            name: "search-skills-github".into(),
            title: Some("Search GitHub for skills".into()),
            description: Some(
                "Searches GitHub for existing SKILL.md files matching the query.".into(),
            ),
            input_schema: Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "query": {
                            "type": "string",
                            "description": "Search query for skills"
                        },
                        "limit": {
                            "type": "integer",
                            "default": 10,
                            "description": "Maximum results to return"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["query"]));
                schema
            }),
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
        Tool {
            name: "search-skills-fuzzy".into(),
            title: Some("Fuzzy search installed skills".into()),
            description: Some(
                "Search installed skills using trigram-based fuzzy matching. \
                 Tolerates typos and finds similar skill names (e.g., 'databas' finds 'database')."
                    .into(),
            ),
            input_schema: Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "query": {
                            "type": "string",
                            "description": "Search query (skill name or partial match)"
                        },
                        "threshold": {
                            "type": "number",
                            "default": 0.3,
                            "minimum": 0.0,
                            "maximum": 1.0,
                            "description": "Similarity threshold (0.0-1.0). Lower = more results, higher = stricter matching"
                        },
                        "limit": {
                            "type": "integer",
                            "default": 10,
                            "description": "Maximum results to return"
                        },
                        "include_description": {
                            "type": "boolean",
                            "default": true,
                            "description": "Also search skill descriptions (not yet implemented - currently matches names only)"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["query"]));
                schema
            }),
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
    ]
}

/// Returns all MCP tools.
///
/// This combines all tool groups and is used by the `list_tools()` handler.
pub(crate) fn all_tools() -> Vec<Tool> {
    let mut tools = Vec::new();
    tools.extend(sync_tools());
    tools.extend(validation_tools());
    tools.extend(dependency_tools());
    tools.extend(recommend_tools());
    tools.extend(metrics_tools());
    tools.extend(trace_tools());
    tools.extend(intelligence_tools());
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_tools_returns_expected_count() {
        let tools = all_tools();
        // 7 sync + 2 validation + 1 dependency + 1 recommend + 1 metrics + 4 trace + 6 intelligence = 22 tools
        assert_eq!(tools.len(), 22);
    }

    #[test]
    fn test_intelligence_tools_count() {
        assert_eq!(intelligence_tools().len(), 6);
    }

    #[test]
    fn test_recommend_tools_count() {
        assert_eq!(recommend_tools().len(), 1);
    }

    #[test]
    fn test_sync_tools_count() {
        assert_eq!(sync_tools().len(), 7);
    }

    #[test]
    fn test_validation_tools_count() {
        assert_eq!(validation_tools().len(), 2);
    }

    #[test]
    fn test_dependency_tools_count() {
        assert_eq!(dependency_tools().len(), 1);
    }

    #[test]
    fn test_metrics_tools_count() {
        assert_eq!(metrics_tools().len(), 1);
    }

    #[test]
    fn test_trace_tools_count() {
        assert_eq!(trace_tools().len(), 4);
    }

    #[test]
    fn test_empty_schema_has_required_fields() {
        let schema = empty_schema();
        assert_eq!(schema.get("type").unwrap(), "object");
        assert!(schema.contains_key("properties"));
        assert!(schema.contains_key("additionalProperties"));
    }

    #[test]
    fn test_tool_names_are_unique() {
        let tools = all_tools();
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        let mut unique_names = names.clone();
        unique_names.sort();
        unique_names.dedup();
        assert_eq!(names.len(), unique_names.len(), "Tool names must be unique");
    }
}
