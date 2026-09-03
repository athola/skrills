//! MCP tool schema definitions for SkillService.
//!
//! This module contains the JSON schema definitions for all MCP tools,
//! organized into logical groups:
//!
//! - Sync tools: cross-agent synchronization (sync-skills, sync-commands, etc.)
//! - Validation tools: skill validation and analysis
//! - Dependency tools: dependency resolution and tracking
//! - Intelligence tools: context tracking, MCP gateway, skill creation
//! - Metrics tools: skill statistics and metrics
//! - Trace tools: skill loading instrumentation
//! - Recommendation tools: skill search and recommendations
//! - Research tools: academic paper search, knowledge graphs, citations, TRIZ

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

/// Returns a standard result output schema with success/message/data fields.
fn result_output_schema() -> Option<Arc<JsonMap<String, serde_json::Value>>> {
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("object"));
    schema.insert(
        "properties".into(),
        json!({
            "success": { "type": "boolean", "description": "Whether the operation succeeded" },
            "message": { "type": "string", "description": "Human-readable result message" },
            "data": { "type": "object", "description": "Operation-specific result data" }
        }),
    );
    schema.insert("required".into(), json!(["success"]));
    Some(Arc::new(schema))
}

/// Returns an array output schema for list operations.
fn array_output_schema(item_desc: &str) -> Option<Arc<JsonMap<String, serde_json::Value>>> {
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("array"));
    schema.insert("items".into(), json!({ "type": "object" }));
    schema.insert("description".into(), json!(item_desc));
    Some(Arc::new(schema))
}

/// Returns the schema for sync tools (from, to, dry_run, force parameters).
fn sync_schema() -> Arc<JsonMap<String, serde_json::Value>> {
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("object"));
    schema.insert(
        "properties".into(),
        json!({
            "from": {
                "type": "string",
                "enum": ["claude", "codex", "copilot", "cursor"],
                "description": "Source agent: 'claude', 'codex', 'copilot', or 'cursor'"
            },
            "to": {
                "type": "string",
                "enum": ["claude", "codex", "copilot", "cursor"],
                "description": "Target agent: 'claude', 'codex', 'copilot', or 'cursor'. Defaults to codex (for claude source) or claude (for others)"
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
/// Tools: sync-from-claude, sync-from-copilot, sync-to-copilot, sync-skills,
/// sync-commands, sync-mcp-servers, sync-preferences, sync-all, sync-status
pub(crate) fn sync_tools() -> Vec<Tool> {
    let schema_empty = empty_schema();
    let sync_schema = sync_schema();

    vec![
        { let mut tool = Tool::new("sync-from-claude", "Copy SKILL.md files from ~/.claude into ~/.codex/skills (Codex discovery root)", schema_empty.clone()).with_title("Copy ~/.claude skills into ~/.codex").with_annotations(ToolAnnotations::default()); tool.output_schema = result_output_schema(); tool },
        Tool::new("sync-from-copilot", "Sync skills and instructions from GitHub Copilot CLI (~/.config/github-copilot) to Claude or Codex.", sync_schema.clone()).with_title("Sync from GitHub Copilot CLI").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-to-copilot", "Sync skills and instructions from Claude or Codex to GitHub Copilot CLI (~/.config/github-copilot).", sync_schema.clone()).with_title("Sync to GitHub Copilot CLI").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-from-cursor", "Sync skills, commands, agents, hooks, rules, and MCP servers from Cursor (~/.cursor) to Claude or Codex.", sync_schema.clone()).with_title("Sync from Cursor IDE").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-to-cursor", "Sync skills, commands, agents, hooks, rules (.mdc), and MCP servers from Claude or Codex to Cursor (~/.cursor).", sync_schema.clone()).with_title("Sync to Cursor IDE").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-skills", "Sync SKILL.md files between Claude, Codex, Copilot, and Cursor. Use --from and --to to specify source and target.", sync_schema.clone()).with_title("Sync skills between agents").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-commands", "Sync slash command definitions between Claude, Codex, Copilot, and Cursor.", sync_schema.clone()).with_title("Sync slash commands between agents").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-mcp-servers", "Sync MCP server configurations between Claude, Codex, Copilot, and Cursor.", sync_schema.clone()).with_title("Sync MCP server configurations").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-preferences", "Sync compatible settings/preferences between Claude, Codex, Copilot, and Cursor.", sync_schema.clone()).with_title("Sync preferences between agents").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-all", "Sync skills, commands, hooks, MCP servers, and preferences between Claude, Codex, Copilot, and Cursor in one operation.", sync_schema.clone()).with_title("Sync all configurations").with_annotations(ToolAnnotations::default()),
        Tool::new("sync-status", "Show what would be synced without making changes (dry run).", sync_schema).with_title("Preview sync changes").with_annotations(ToolAnnotations::default()),
    ]
}

/// Returns validation and analysis tools.
///
/// Tools: validate-skills, analyze-skills
pub(crate) fn validation_tools() -> Vec<Tool> {
    vec![
        { let mut tool = Tool::new("validate-skills", "Validate skills for Claude Code, Codex, and/or Copilot CLI compatibility. Returns validation errors and warnings.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "target": {
                            "type": "string",
                            "enum": ["claude", "codex", "copilot", "both", "all"],
                            "default": "both",
                            "description": "Validation target: 'claude', 'codex', 'copilot', 'both' (claude+codex), or 'all'"
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
            })).with_title("Validate skills for CLI compatibility").with_annotations(ToolAnnotations::default()); tool.output_schema = array_output_schema("Validation results per skill"); tool },
        Tool::new("analyze-skills", "Analyze skills for token usage, dependencies, and optimization suggestions. Returns detailed analysis with quality scores.", Arc::new({
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
            })).with_title("Analyze skills for token usage and optimization").with_annotations(ToolAnnotations::default()),
        Tool::new("skill-diff", "Compare a skill across Claude, Codex, and Copilot to identify differences in content and frontmatter. \
                 Shows unified diff, frontmatter variations, and token count differences.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "name": {
                            "type": "string",
                            "description": "Skill name to compare (e.g., 'commit', 'review-pr')"
                        },
                        "context_lines": {
                            "type": "integer",
                            "default": 3,
                            "description": "Number of context lines to show around differences"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["name"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Compare skill versions across CLIs").with_annotations(ToolAnnotations::default()),
    ]
}

/// Returns dependency resolution tools.
///
/// Tools: resolve-dependencies
pub(crate) fn dependency_tools() -> Vec<Tool> {
    vec![Tool::new("resolve-dependencies", "Get transitive dependencies or dependents for a skill.", Arc::new({
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
        })).with_title("Resolve skill dependencies").with_annotations(ToolAnnotations::default())]
}

/// Returns recommendation tools.
///
/// Tools: recommend-skills
pub(crate) fn recommend_tools() -> Vec<Tool> {
    vec![Tool::new("recommend-skills", "Recommends related skills based on dependency relationships. Given a skill URI, suggests dependencies, dependents, and sibling skills (those sharing common dependencies).", Arc::new({
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
        })).with_title("Get skill recommendations").with_annotations(ToolAnnotations::default())]
}

/// Returns metrics tools.
///
/// Tools: skill-metrics
pub(crate) fn metrics_tools() -> Vec<Tool> {
    vec![Tool::new("skill-metrics", "Returns aggregate statistics about discovered skills including counts, quality distribution, dependency patterns, and token usage.", Arc::new({
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
        })).with_title("Get skill statistics and metrics").with_annotations(ToolAnnotations::default())]
}

/// Returns skill trace and instrumentation tools.
///
/// Tools: skill-loading-status, enable-skill-trace, disable-skill-trace, skill-loading-selftest
pub(crate) fn trace_tools() -> Vec<Tool> {
    vec![
        Tool::new("skill-loading-status", "Checks skill roots on disk and reports whether trace/probe skills are installed and whether skill files are instrumented with skrills markers.", Arc::new({
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
            })).with_title("Skill loading status (filesystem + instrumentation)").with_annotations(ToolAnnotations::default()),
        Tool::new("enable-skill-trace", "Installs skrills trace/probe skills and (optionally) instruments SKILL.md files with markers so the trace skill can report which skills were loaded.", Arc::new({
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
            })).with_title("Enable deterministic skill tracing").with_annotations(ToolAnnotations::default()),
        Tool::new("disable-skill-trace", "Removes the skrills trace/probe skill directories from primary Claude/Codex skill roots (does not remove instrumentation markers).", Arc::new({
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
            })).with_title("Disable skill tracing").with_annotations(ToolAnnotations::default()),
        Tool::new("skill-loading-selftest", "Ensures the probe skill exists and returns a one-shot probe line + expected response to confirm skills are loading in the current session.", Arc::new({
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
            })).with_title("Skill loading selftest (probe)").with_annotations(ToolAnnotations::default()),
    ]
}

/// Returns intelligent recommendation and skill creation tools.
///
/// Tools: recommend-skills-smart, analyze-project-context, suggest-new-skills,
/// create-skill, search-skills-github
pub(crate) fn intelligence_tools() -> Vec<Tool> {
    vec![
        Tool::new("recommend-skills-smart", "Enhanced recommendations combining dependency relationships, usage patterns, \
                 and project context. Returns scored recommendations with explanations.", Arc::new({
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
            })).with_title("Smart skill recommendations").with_annotations(ToolAnnotations::default()),
        Tool::new("analyze-project-context", "Analyzes the current project to build a context profile including \
                 languages, dependencies, frameworks, and keywords.", Arc::new({
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
            })).with_title("Analyze project context").with_annotations(ToolAnnotations::default()),
        Tool::new("suggest-new-skills", "Identifies gaps in your skill library based on project context \
                 and usage patterns, suggesting new skills to create.", Arc::new({
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
            })).with_title("Suggest skills to create").with_annotations(ToolAnnotations::default()),
        Tool::new("create-skill", "Creates a new skill via GitHub search, LLM generation, or both. \
                 Default behavior: search GitHub first, then generate if not found.", Arc::new({
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
                            "enum": ["github", "llm", "both", "empirical"],
                            "default": "both",
                            "description": "Creation method: 'github' (search), 'llm' (generate), 'both', or 'empirical' (session patterns)"
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
            })).with_title("Create a new skill").with_annotations(ToolAnnotations::default()),
        Tool::new("search-skills-github", "Searches GitHub for existing SKILL.md files matching the query.", Arc::new({
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
            })).with_title("Search GitHub for skills").with_annotations(ToolAnnotations::default()),
        Tool::new("search-skills-fuzzy", "Search installed skills using trigram-based fuzzy matching. \
                 Tolerates typos and finds similar skill names (e.g., 'databas' finds 'database'). \
                 Aligns with CLI command: `skrills search-skills`.", Arc::new({
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
                            "description": "Also search skill descriptions for richer matching. Set to false for name-only matching (faster)."
                        }
                    }),
                );
                schema.insert("required".into(), json!(["query"]));
                schema
            })).with_title("Search installed skills").with_annotations(ToolAnnotations::default()),
    ]
}

/// Returns research tools for academic paper search, knowledge graphs, and TRIZ.
///
/// Tools: search-papers, search-discussions, resolve-doi, fetch-pdf,
/// query-knowledge-graph, add-knowledge-node, link-knowledge, track-citations,
/// resolve-contradiction
pub(crate) fn research_tools() -> Vec<Tool> {
    vec![
        // --- #168 Tools (Research API) ---
        Tool::new("search-papers", "Search for academic papers across Semantic Scholar, arXiv, and OpenAlex. Deduplicates results by DOI.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "query": {
                            "type": "string",
                            "description": "Search query for academic papers"
                        },
                        "limit": {
                            "type": "integer",
                            "default": 10,
                            "maximum": 100,
                            "description": "Maximum number of results to return (default 10, max 100)"
                        },
                        "sources": {
                            "type": "array",
                            "items": {
                                "type": "string",
                                "enum": ["arxiv", "semantic_scholar", "openalex"]
                            },
                            "description": "Paper sources to search (defaults to all)"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["query"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Search academic papers").with_annotations(ToolAnnotations::default()),
        Tool::new("search-discussions", "Search Hacker News for community discussions about a topic.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "query": {
                            "type": "string",
                            "description": "Search query for discussions"
                        },
                        "limit": {
                            "type": "integer",
                            "default": 10,
                            "maximum": 100,
                            "description": "Maximum number of results to return"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["query"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Search community discussions").with_annotations(ToolAnnotations::default()),
        Tool::new("resolve-doi", "Resolve a DOI to full metadata via CrossRef, with open-access PDF URL from Unpaywall.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "doi": {
                            "type": "string",
                            "description": "DOI to resolve (e.g., '10.1234/example')"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["doi"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Resolve DOI metadata").with_annotations(ToolAnnotations::default()),
        Tool::new("fetch-pdf", "Download the open-access PDF for a DOI via Unpaywall and cache it locally. Returns the local file path.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "doi": {
                            "type": "string",
                            "description": "DOI of the paper to fetch PDF for"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["doi"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Download and cache academic PDF").with_annotations(ToolAnnotations::default()),
        // --- #169 Tools (Advanced Features) ---
        Tool::new("query-knowledge-graph", "Search nodes or traverse edges in the research knowledge graph. Provide query to search, or node_id to get connections.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "query": {
                            "type": "string",
                            "description": "Text query to search nodes"
                        },
                        "node_id": {
                            "type": "string",
                            "description": "Node ID to get connections for"
                        },
                        "direction": {
                            "type": "string",
                            "enum": ["from", "to", "both"],
                            "description": "Edge traversal direction when using node_id"
                        },
                        "kind": {
                            "type": "string",
                            "enum": ["topic", "paper", "implementation", "discussion"],
                            "description": "Filter by node kind"
                        }
                    }),
                );
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Search and traverse knowledge graph").with_annotations(ToolAnnotations::default()),
        Tool::new("add-knowledge-node", "Add a node to the persistent research knowledge graph.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "id": {
                            "type": "string",
                            "description": "Unique node identifier"
                        },
                        "kind": {
                            "type": "string",
                            "enum": ["topic", "paper", "implementation", "discussion"],
                            "description": "Node kind"
                        },
                        "label": {
                            "type": "string",
                            "description": "Human-readable node label"
                        },
                        "metadata": {
                            "type": "object",
                            "description": "Optional metadata for the node"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["id", "kind", "label"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Add a node to the knowledge graph").with_annotations(ToolAnnotations::default()),
        Tool::new("link-knowledge", "Create a directed edge between two nodes in the knowledge graph.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "source_id": {
                            "type": "string",
                            "description": "Source node ID"
                        },
                        "target_id": {
                            "type": "string",
                            "description": "Target node ID"
                        },
                        "kind": {
                            "type": "string",
                            "enum": ["cites", "implements", "contradicts", "extends", "analogous_to"],
                            "description": "Edge relationship kind"
                        },
                        "weight": {
                            "type": "number",
                            "default": 1.0,
                            "description": "Edge weight (default 1.0)"
                        },
                        "metadata": {
                            "type": "object",
                            "description": "Optional metadata for the edge"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["source_id", "target_id", "kind"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Connect nodes in the knowledge graph").with_annotations(ToolAnnotations::default()),
        Tool::new("track-citations", "Track a paper for citation monitoring, or query forward/backward citations.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "paper_id": {
                            "type": "string",
                            "description": "Paper identifier (e.g., Semantic Scholar ID)"
                        },
                        "title": {
                            "type": "string",
                            "description": "Paper title for display"
                        },
                        "doi": {
                            "type": "string",
                            "description": "Optional DOI for cross-referencing"
                        },
                        "action": {
                            "type": "string",
                            "enum": ["track", "forward", "backward"],
                            "default": "track",
                            "description": "Action: 'track' to monitor, 'forward' for citing papers, 'backward' for referenced papers"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["paper_id"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("Track paper citations").with_annotations(ToolAnnotations::default()),
        Tool::new("resolve-contradiction", "Apply TRIZ inventive principles to resolve a contradiction between two parameters. Returns applicable principles with software examples.", Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "improve": {
                            "type": "string",
                            "enum": [
                                "performance", "reliability", "maintainability", "scalability",
                                "security", "usability", "testability", "deployability",
                                "cost_efficiency", "development_speed", "code_complexity",
                                "memory_usage", "latency", "throughput", "availability"
                            ],
                            "description": "Parameter to improve"
                        },
                        "degrades": {
                            "type": "string",
                            "enum": [
                                "performance", "reliability", "maintainability", "scalability",
                                "security", "usability", "testability", "deployability",
                                "cost_efficiency", "development_speed", "code_complexity",
                                "memory_usage", "latency", "throughput", "availability"
                            ],
                            "description": "Parameter that would degrade"
                        }
                    }),
                );
                schema.insert("required".into(), json!(["improve", "degrades"]));
                schema.insert("additionalProperties".into(), json!(false));
                schema
            })).with_title("TRIZ contradiction resolution").with_annotations(ToolAnnotations::default()),
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
    tools.extend(research_tools());
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_tools_returns_expected_count() {
        let tools = all_tools();
        // 11 sync + 3 validation + 1 dependency + 1 recommend + 1 metrics + 4 trace + 6 intelligence + 9 research = 36 tools
        assert_eq!(tools.len(), 36);
    }

    #[test]
    fn test_research_tools_count() {
        assert_eq!(research_tools().len(), 9);
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
        assert_eq!(sync_tools().len(), 11);
    }

    #[test]
    fn test_validation_tools_count() {
        assert_eq!(validation_tools().len(), 3);
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
