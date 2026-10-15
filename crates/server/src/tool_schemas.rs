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

/// Builds a tool with the four fields every definition below sets, so each one
/// stays a single expression rustfmt can break.
fn tool(
    name: &'static str,
    title: &'static str,
    description: &'static str,
    input_schema: Arc<JsonMap<String, serde_json::Value>>,
) -> Tool {
    Tool::new(name, description, input_schema)
        .with_title(title)
        .with_annotations(ToolAnnotations::default())
}

/// Input schema for `sync-from-claude`, whose handler reads
/// `include_marketplace`.
fn sync_from_claude_input_schema() -> Arc<JsonMap<String, serde_json::Value>> {
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("object"));
    schema.insert(
        "properties".into(),
        json!({
            "include_marketplace": {
                "type": "boolean",
                "default": false,
                "description": "Also copy skills under ~/.claude/plugins/marketplaces"
            }
        }),
    );
    schema.insert("additionalProperties".into(), json!(false));
    Arc::new(schema)
}

/// Output schema for `sync-from-claude`: the copy report plus priority
/// metadata, matching the handler's structured content.
fn sync_from_claude_output_schema() -> Arc<JsonMap<String, serde_json::Value>> {
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("object"));
    schema.insert(
        "properties".into(),
        json!({
            "report": {
                "type": "object",
                "properties": {
                    "copied": { "type": "integer", "description": "Skills written to ~/.agents/skills" },
                    "skipped": { "type": "integer", "description": "Skills already up to date or not written" },
                    "warnings": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Problems that may stop Codex loading the skills"
                    }
                },
                "required": ["copied", "skipped", "warnings"]
            },
            "_meta": {
                "type": "object",
                "properties": {
                    "priority": { "type": "array", "items": { "type": "string" } },
                    "priority_rank_by_source": { "type": "object" }
                }
            }
        }),
    );
    schema.insert("required".into(), json!(["report"]));
    Arc::new(schema)
}

/// Output schema for `validate-skills`, matching the handler's structured
/// content: a summary object with one entry per validated skill.
fn validate_skills_output_schema() -> Arc<JsonMap<String, serde_json::Value>> {
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("object"));
    schema.insert(
        "properties".into(),
        json!({
            "total": { "type": "integer", "description": "Number of skills reported" },
            "target": { "type": "string", "description": "Validation target used" },
            "autofix": { "type": "boolean" },
            "autofixed": { "type": "integer", "description": "Skills changed by autofix" },
            "results": {
                "type": "array",
                "items": { "type": "object" },
                "description": "Validation results per skill"
            },
            "check_dependencies": { "type": "boolean" },
            "total_dependency_issues": { "type": "integer" }
        }),
    );
    schema.insert(
        "required".into(),
        json!(["total", "target", "autofix", "autofixed", "results"]),
    );
    Arc::new(schema)
}

/// Returns the schema for a sync tool that reads `keys` (a subset of from,
/// to, dry_run, include_marketplace and skip_existing_commands).
fn sync_schema(keys: &[&str]) -> Arc<JsonMap<String, serde_json::Value>> {
    let all = json!({
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
        "include_marketplace": {
            "type": "boolean",
            "default": false,
            "description": "Also read skills and commands under ~/.claude/plugins/marketplaces"
        },
        "skip_existing_commands": {
            "type": "boolean",
            "default": false,
            "description": "Leave commands that already exist in the target unchanged"
        }
    });
    let properties: JsonMap<String, serde_json::Value> = keys
        .iter()
        .filter_map(|key| all.get(*key).map(|v| ((*key).to_string(), v.clone())))
        .collect();
    let mut schema = JsonMap::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), serde_json::Value::Object(properties));
    schema.insert("additionalProperties".into(), json!(false));
    Arc::new(schema)
}

/// Returns sync-related tools.
///
/// Tools: sync-from-claude, sync-from-copilot, sync-to-copilot, sync-skills,
/// sync-commands, sync-mcp-servers, sync-preferences, sync-all, sync-status
pub(crate) fn sync_tools() -> Vec<Tool> {
    const PEER_FROM: &[&str] = &["from", "dry_run"];
    const PEER_TO: &[&str] = &["to", "dry_run"];
    const BETWEEN: &[&str] = &["from", "to", "dry_run"];
    const WITH_COMMANDS: &[&str] = &[
        "from",
        "to",
        "dry_run",
        "include_marketplace",
        "skip_existing_commands",
    ];

    vec![
        tool(
            "sync-from-claude",
            "Copy ~/.claude skills into ~/.codex",
            "Copy ~/.claude skills into ~/.agents/skills, where Codex loads user skills",
            sync_from_claude_input_schema(),
        )
        .with_raw_output_schema(sync_from_claude_output_schema()),
        tool(
            "sync-from-copilot",
            "Sync from GitHub Copilot CLI",
            "Sync skills and instructions from GitHub Copilot CLI \
             (~/.config/github-copilot) to Claude or Codex.",
            sync_schema(PEER_TO),
        ),
        tool(
            "sync-to-copilot",
            "Sync to GitHub Copilot CLI",
            "Sync skills and instructions from Claude or Codex to GitHub Copilot CLI \
             (~/.config/github-copilot).",
            sync_schema(PEER_FROM),
        ),
        tool(
            "sync-from-cursor",
            "Sync from Cursor IDE",
            "Sync skills, commands, agents, hooks, rules, and MCP servers from Cursor \
             (~/.cursor) to Claude or Codex.",
            sync_schema(PEER_TO),
        ),
        tool(
            "sync-to-cursor",
            "Sync to Cursor IDE",
            "Sync skills, commands, agents, hooks, rules (.mdc), and MCP servers from \
             Claude or Codex to Cursor (~/.cursor).",
            sync_schema(PEER_FROM),
        ),
        tool(
            "sync-skills",
            "Sync skills between agents",
            "Sync SKILL.md files between Claude, Codex, Copilot, and Cursor. Use --from \
             and --to to specify source and target.",
            sync_schema(&["from", "to", "dry_run", "include_marketplace"]),
        ),
        tool(
            "sync-commands",
            "Sync slash commands between agents",
            "Sync slash command definitions between Claude, Codex, Copilot, and Cursor.",
            sync_schema(WITH_COMMANDS),
        ),
        tool(
            "sync-mcp-servers",
            "Sync MCP server configurations",
            "Sync MCP server configurations between Claude, Codex, Copilot, and Cursor.",
            sync_schema(BETWEEN),
        ),
        tool(
            "sync-preferences",
            "Sync preferences between agents",
            "Sync compatible settings/preferences between Claude, Codex, Copilot, and \
             Cursor.",
            sync_schema(BETWEEN),
        ),
        tool(
            "sync-all",
            "Sync all configurations",
            "Sync skills, commands, hooks, MCP servers, and preferences between Claude, \
             Codex, Copilot, and Cursor in one operation.",
            sync_schema(WITH_COMMANDS),
        ),
        tool(
            "sync-status",
            "Preview sync changes",
            "Show what would be synced without making changes (dry run).",
            sync_schema(&["from", "to"]),
        ),
    ]
}

/// Returns validation and analysis tools.
///
/// Tools: validate-skills, analyze-skills
pub(crate) fn validation_tools() -> Vec<Tool> {
    vec![
        tool(
            "validate-skills",
            "Validate skills for CLI compatibility",
            "Validate skills for Claude Code, Codex, and/or Copilot CLI compatibility. \
             Returns validation errors and warnings.",
            Arc::new({
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
            }),
        )
        .with_raw_output_schema(validate_skills_output_schema()),
        tool(
            "analyze-skills",
            "Analyze skills for token usage and optimization",
            "Analyze skills for token usage, dependencies, and optimization suggestions. \
             Returns detailed analysis with quality scores.",
            Arc::new({
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
        ),
        tool(
            "skill-diff",
            "Compare skill versions across CLIs",
            "Compare a skill across Claude, Codex, and Copilot to identify differences in \
             content and frontmatter. Shows unified diff, frontmatter variations, and \
             token count differences.",
            Arc::new({
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
            }),
        ),
    ]
}

/// Returns dependency resolution tools.
///
/// Tools: resolve-dependencies
pub(crate) fn dependency_tools() -> Vec<Tool> {
    vec![tool(
        "resolve-dependencies",
        "Resolve skill dependencies",
        "Get transitive dependencies or dependents for a skill.",
        Arc::new({
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
    )]
}

/// Returns recommendation tools.
///
/// Tools: recommend-skills
pub(crate) fn recommend_tools() -> Vec<Tool> {
    vec![tool(
        "recommend-skills",
        "Get skill recommendations",
        "Recommends related skills based on dependency relationships. Given a skill \
             URI, suggests dependencies, dependents, and sibling skills (those sharing \
             common dependencies).",
        Arc::new({
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
    )]
}

/// Returns metrics tools.
///
/// Tools: skill-metrics
pub(crate) fn metrics_tools() -> Vec<Tool> {
    vec![tool(
        "skill-metrics",
        "Get skill statistics and metrics",
        "Returns aggregate statistics about discovered skills including counts, \
             quality distribution, dependency patterns, and token usage.",
        Arc::new({
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
    )]
}

/// Returns skill trace and instrumentation tools.
///
/// Tools: skill-loading-status, enable-skill-trace, disable-skill-trace, skill-loading-selftest
pub(crate) fn trace_tools() -> Vec<Tool> {
    vec![
        tool(
            "skill-loading-status",
            "Skill loading status (filesystem + instrumentation)",
            "Checks skill roots on disk and reports whether trace/probe skills are \
             installed and whether skill files are instrumented with skrills markers.",
            Arc::new({
                let mut schema = JsonMap::new();
                schema.insert("type".into(), json!("object"));
                schema.insert(
                    "properties".into(),
                    json!({
                        "target": { "type": "string", "description": "Target client: claude, codex, or both", "default": "both" },
                        "include_mirror": { "type": "boolean", "default": true, "description": "Include ~/.codex/skills-mirror when target includes codex" },
                        "include_agent": { "type": "boolean", "default": true, "description": "Include ~/.agents/skills" },
                        "include_cache": { "type": "boolean", "default": false, "description": "Include ~/.claude/plugins/cache when target includes claude" },
                        "include_marketplace": { "type": "boolean", "default": false, "description": "Include ~/.claude/plugins/marketplaces when target includes claude" }
                    }),
                );
                schema.insert("additionalProperties".into(), json!(false));
                schema
            }),
        ),
        tool(
            "enable-skill-trace",
            "Enable deterministic skill tracing",
            "Installs skrills trace/probe skills and (optionally) instruments SKILL.md \
             files with markers so the trace skill can report which skills were loaded.",
            Arc::new({
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
                        "include_agent": { "type": "boolean", "default": true, "description": "Include ~/.agents/skills when instrumenting" },
                        "include_cache": { "type": "boolean", "default": false, "description": "Include ~/.claude/plugins/cache when instrumenting claude" },
                        "include_marketplace": { "type": "boolean", "default": false, "description": "Include ~/.claude/plugins/marketplaces when instrumenting claude" }
                    }),
                );
                schema.insert("additionalProperties".into(), json!(false));
                schema
            }),
        ),
        tool(
            "disable-skill-trace",
            "Disable skill tracing",
            "Removes the skrills trace/probe skill directories from primary Claude/Codex \
             skill roots (does not remove instrumentation markers).",
            Arc::new({
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
        ),
        tool(
            "skill-loading-selftest",
            "Skill loading selftest (probe)",
            "Ensures the probe skill exists and returns a one-shot probe line + expected \
             response to confirm skills are loading in the current session.",
            Arc::new({
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
        ),
    ]
}

/// Returns intelligent recommendation and skill creation tools.
///
/// Tools: recommend-skills-smart, analyze-project-context, suggest-new-skills,
/// create-skill, search-skills-github
pub(crate) fn intelligence_tools() -> Vec<Tool> {
    vec![
        tool(
            "recommend-skills-smart",
            "Smart skill recommendations",
            "Enhanced recommendations combining dependency relationships, usage patterns, \
                 and project context. Returns scored recommendations with explanations.",
            Arc::new({
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
        ),
        tool(
            "analyze-project-context",
            "Analyze project context",
            "Analyzes the current project to build a context profile including \
                 languages, dependencies, frameworks, and keywords.",
            Arc::new({
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
                        },
                        "max_languages": {
                            "type": "integer",
                            "default": 10,
                            "description": "Report at most this many languages, by file count (0 for all)"
                        }
                    }),
                );
                schema.insert("additionalProperties".into(), json!(false));
                schema
            }),
        ),
        tool(
            "suggest-new-skills",
            "Suggest skills to create",
            "Identifies gaps in your skill library based on project context \
                 and usage patterns, suggesting new skills to create.",
            Arc::new({
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
        ),
        tool(
            "create-skill",
            "Create a new skill",
            "Creates a new skill via GitHub search, LLM generation, or both. \
                 Default behavior: search GitHub first, then generate if not found.",
            Arc::new({
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
                        "project_dir": {
                            "type": "string",
                            "description": "Project directory whose context informs generation"
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
        ),
        tool(
            "search-skills-github",
            "Search GitHub for skills",
            "Searches GitHub for existing SKILL.md files matching the query.",
            Arc::new({
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
        ),
        tool(
            "search-skills-fuzzy",
            "Search installed skills",
            "Search installed skills using trigram-based fuzzy matching. \
                 Tolerates typos and finds similar skill names (e.g., 'databas' finds 'database'). \
                 Aligns with CLI command: `skrills search-skills`.",
            Arc::new({
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
            }),
        ),
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
        tool(
            "search-papers",
            "Search academic papers",
            "Search for academic papers across Semantic Scholar, arXiv, and OpenAlex. \
             Deduplicates results by DOI.",
            Arc::new({
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
            }),
        ),
        tool(
            "search-discussions",
            "Search community discussions",
            "Search Hacker News for community discussions about a topic.",
            Arc::new({
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
            }),
        ),
        tool(
            "resolve-doi",
            "Resolve DOI metadata",
            "Resolve a DOI to full metadata via CrossRef, with open-access PDF URL from \
             Unpaywall.",
            Arc::new({
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
            }),
        ),
        tool(
            "fetch-pdf",
            "Download and cache academic PDF",
            "Download the open-access PDF for a DOI via Unpaywall and cache it locally. \
             Returns the local file path.",
            Arc::new({
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
            }),
        ),
        // --- #169 Tools (Advanced Features) ---
        tool(
            "query-knowledge-graph",
            "Search and traverse knowledge graph",
            "Search nodes or traverse edges in the research knowledge graph. Provide \
             query to search, or node_id to get connections.",
            Arc::new({
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
            }),
        ),
        tool(
            "add-knowledge-node",
            "Add a node to the knowledge graph",
            "Add a node to the persistent research knowledge graph.",
            Arc::new({
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
            }),
        ),
        tool(
            "link-knowledge",
            "Connect nodes in the knowledge graph",
            "Create a directed edge between two nodes in the knowledge graph.",
            Arc::new({
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
            }),
        ),
        tool(
            "track-citations",
            "Track paper citations",
            "Track a paper for citation monitoring, or query forward/backward citations.",
            Arc::new({
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
            }),
        ),
        tool(
            "resolve-contradiction",
            "TRIZ contradiction resolution",
            "Apply TRIZ inventive principles to resolve a contradiction between two \
             parameters. Returns applicable principles with software examples.",
            Arc::new({
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
            }),
        ),
    ]
}

/// Returns all MCP tools.
///
/// This combines all tool groups and is used by the `list_tools()` handler.
pub fn all_tools() -> Vec<Tool> {
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
    fn every_input_schema_declares_an_object_type() {
        // Codex rejects a tool whose input schema has no `type` during its
        // MCP -> OpenAI conversion ("missing field `type`").
        for tool in all_tools() {
            assert_eq!(
                tool.input_schema.get("type").and_then(|t| t.as_str()),
                Some("object"),
                "{} input schema lacks type: object",
                tool.name
            );
        }
    }

    /// The builder rewrite defaults every optional field to `None`, so a
    /// dropped `.with_title(..)` or `.with_annotations(..)` would pass the
    /// count tests above while silently degrading the client-facing listing.
    #[test]
    fn every_tool_carries_title_and_annotations() {
        for tool in all_tools() {
            assert!(tool.title.is_some(), "{} lost its title", tool.name);
            assert!(
                tool.annotations.is_some(),
                "{} lost its annotations",
                tool.name
            );
        }
    }

    /// The client-facing listing, in order, as it was before the definitions
    /// moved onto the `tool` helper. The counts above would survive a mistyped
    /// name or a title landing in the description slot, so pin the table.
    const EXPECTED_LISTING: &[(&str, &str, bool)] = &[
        (
            "sync-from-claude",
            "Copy ~/.claude skills into ~/.codex",
            true,
        ),
        ("sync-from-copilot", "Sync from GitHub Copilot CLI", false),
        ("sync-to-copilot", "Sync to GitHub Copilot CLI", false),
        ("sync-from-cursor", "Sync from Cursor IDE", false),
        ("sync-to-cursor", "Sync to Cursor IDE", false),
        ("sync-skills", "Sync skills between agents", false),
        ("sync-commands", "Sync slash commands between agents", false),
        ("sync-mcp-servers", "Sync MCP server configurations", false),
        ("sync-preferences", "Sync preferences between agents", false),
        ("sync-all", "Sync all configurations", false),
        ("sync-status", "Preview sync changes", false),
        (
            "validate-skills",
            "Validate skills for CLI compatibility",
            true,
        ),
        (
            "analyze-skills",
            "Analyze skills for token usage and optimization",
            false,
        ),
        ("skill-diff", "Compare skill versions across CLIs", false),
        ("resolve-dependencies", "Resolve skill dependencies", false),
        ("recommend-skills", "Get skill recommendations", false),
        ("skill-metrics", "Get skill statistics and metrics", false),
        (
            "skill-loading-status",
            "Skill loading status (filesystem + instrumentation)",
            false,
        ),
        (
            "enable-skill-trace",
            "Enable deterministic skill tracing",
            false,
        ),
        ("disable-skill-trace", "Disable skill tracing", false),
        (
            "skill-loading-selftest",
            "Skill loading selftest (probe)",
            false,
        ),
        (
            "recommend-skills-smart",
            "Smart skill recommendations",
            false,
        ),
        ("analyze-project-context", "Analyze project context", false),
        ("suggest-new-skills", "Suggest skills to create", false),
        ("create-skill", "Create a new skill", false),
        ("search-skills-github", "Search GitHub for skills", false),
        ("search-skills-fuzzy", "Search installed skills", false),
        ("search-papers", "Search academic papers", false),
        ("search-discussions", "Search community discussions", false),
        ("resolve-doi", "Resolve DOI metadata", false),
        ("fetch-pdf", "Download and cache academic PDF", false),
        (
            "query-knowledge-graph",
            "Search and traverse knowledge graph",
            false,
        ),
        (
            "add-knowledge-node",
            "Add a node to the knowledge graph",
            false,
        ),
        (
            "link-knowledge",
            "Connect nodes in the knowledge graph",
            false,
        ),
        ("track-citations", "Track paper citations", false),
        (
            "resolve-contradiction",
            "TRIZ contradiction resolution",
            false,
        ),
    ];

    #[test]
    fn listing_matches_the_names_titles_and_output_schemas_it_shipped_with() {
        let listing: Vec<(String, String, bool)> = all_tools()
            .iter()
            .map(|tool| {
                (
                    tool.name.to_string(),
                    tool.title.clone().unwrap_or_default(),
                    tool.output_schema.is_some(),
                )
            })
            .collect();
        let expected: Vec<(String, String, bool)> = EXPECTED_LISTING
            .iter()
            .map(|(name, title, has_output)| {
                ((*name).to_string(), (*title).to_string(), *has_output)
            })
            .collect();
        assert_eq!(listing, expected);
    }

    /// Only two tools set an output schema. Pin the exact set so neither call
    /// can be dropped, and no tool gains one by accident.
    #[test]
    fn output_schema_is_set_on_exactly_the_two_structured_tools() {
        let mut with_schema: Vec<String> = all_tools()
            .iter()
            .filter(|t| t.output_schema.is_some())
            .map(|t| t.name.to_string())
            .collect();
        with_schema.sort_unstable();
        assert_eq!(with_schema, ["sync-from-claude", "validate-skills"]);
    }

    /// The argument keys each handler reads. Strict clients honour
    /// `additionalProperties: false`, so a key missing from the schema can
    /// never be sent, and a property no handler reads is a false promise.
    const HANDLER_KEYS: &[(&str, &[&str])] = &[
        ("sync-from-claude", &["include_marketplace"]),
        ("sync-from-copilot", &["to", "dry_run"]),
        ("sync-to-copilot", &["from", "dry_run"]),
        ("sync-from-cursor", &["to", "dry_run"]),
        ("sync-to-cursor", &["from", "dry_run"]),
        (
            "sync-skills",
            &["from", "to", "dry_run", "include_marketplace"],
        ),
        (
            "sync-commands",
            &[
                "from",
                "to",
                "dry_run",
                "include_marketplace",
                "skip_existing_commands",
            ],
        ),
        ("sync-mcp-servers", &["from", "to", "dry_run"]),
        ("sync-preferences", &["from", "to", "dry_run"]),
        (
            "sync-all",
            &[
                "from",
                "to",
                "dry_run",
                "include_marketplace",
                "skip_existing_commands",
            ],
        ),
        ("sync-status", &["from", "to"]),
        (
            "validate-skills",
            &["target", "errors_only", "autofix", "check_dependencies"],
        ),
        ("analyze-skills", &["min_tokens", "include_suggestions"]),
        ("skill-diff", &["name", "context_lines"]),
        ("resolve-dependencies", &["uri", "direction", "transitive"]),
        ("recommend-skills", &["uri", "limit", "include_quality"]),
        ("skill-metrics", &["include_validation"]),
        (
            "skill-loading-status",
            &[
                "target",
                "include_mirror",
                "include_agent",
                "include_cache",
                "include_marketplace",
            ],
        ),
        (
            "enable-skill-trace",
            &[
                "target",
                "instrument",
                "backup",
                "dry_run",
                "include_mirror",
                "include_agent",
                "include_cache",
                "include_marketplace",
            ],
        ),
        ("disable-skill-trace", &["target", "dry_run"]),
        ("skill-loading-selftest", &["target", "dry_run"]),
        (
            "recommend-skills-smart",
            &[
                "uri",
                "prompt",
                "project_dir",
                "limit",
                "include_usage",
                "include_context",
            ],
        ),
        (
            "analyze-project-context",
            &[
                "project_dir",
                "include_git",
                "commit_limit",
                "max_languages",
            ],
        ),
        ("suggest-new-skills", &["project_dir", "focus_areas"]),
        (
            "create-skill",
            &[
                "name",
                "description",
                "method",
                "target_dir",
                "dry_run",
                "project_dir",
            ],
        ),
        ("search-skills-github", &["query", "limit"]),
        (
            "search-skills-fuzzy",
            &["query", "threshold", "limit", "include_description"],
        ),
        ("search-papers", &["query", "limit", "sources"]),
        ("search-discussions", &["query", "limit"]),
        ("resolve-doi", &["doi"]),
        ("fetch-pdf", &["doi"]),
        (
            "query-knowledge-graph",
            &["query", "node_id", "direction", "kind"],
        ),
        ("add-knowledge-node", &["id", "kind", "label", "metadata"]),
        (
            "link-knowledge",
            &["source_id", "target_id", "kind", "weight", "metadata"],
        ),
        ("track-citations", &["paper_id", "title", "doi", "action"]),
        ("resolve-contradiction", &["improve", "degrades"]),
    ];

    #[test]
    fn input_schemas_list_exactly_the_keys_their_handlers_read() {
        let tools = all_tools();
        let listed: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
        let covered: Vec<&str> = HANDLER_KEYS.iter().map(|(name, _)| *name).collect();
        assert_eq!(listed, covered, "HANDLER_KEYS must cover every tool");
        for (tool, (_, keys)) in tools.iter().zip(HANDLER_KEYS) {
            let mut properties: Vec<&str> = tool
                .input_schema
                .get("properties")
                .and_then(|p| p.as_object())
                .map(|p| p.keys().map(String::as_str).collect())
                .unwrap_or_default();
            properties.sort_unstable();
            let mut expected = keys.to_vec();
            expected.sort_unstable();
            assert_eq!(properties, expected, "{}", tool.name);
        }
    }

    /// `call_tool` checks subagent names first, then core tools; a gateway
    /// or subagent tool sharing a core name would shadow or be shadowed.
    #[test]
    fn core_gateway_and_subagent_tool_names_do_not_collide() {
        let mut names: Vec<String> = all_tools()
            .iter()
            .chain(crate::mcp_gateway::mcp_gateway_tools().iter())
            .map(|t| t.name.to_string())
            .collect();
        #[cfg(feature = "subagents")]
        if let Ok(service) = skrills_subagents::SubagentService::new() {
            names.extend(service.tools().iter().map(|t| t.name.to_string()));
        }
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate tool names: {names:?}");
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
