//! MCP Gateway tool implementations.

use super::registry::McpToolEntry;
use super::stats::ContextStatsSnapshot;
use anyhow::Result;
use rmcp::model::{CallToolResult, Content, Tool, ToolAnnotations};
use serde_json::{json, Map as JsonMap, Value};
use std::sync::Arc;

/// Tool names handled by the MCP gateway.
pub const MCP_GATEWAY_TOOL_NAMES: &[&str] = &[
    "list-mcp-tools",
    "list_mcp_tools",
    "describe-mcp-tool",
    "describe_mcp_tool",
    "get-context-stats",
    "get_context_stats",
];

/// Helper to create an Arc-wrapped schema.
fn schema(props: Value) -> Arc<JsonMap<String, Value>> {
    let mut map = JsonMap::new();
    map.insert("type".into(), json!("object"));
    map.insert("properties".into(), props);
    map.insert("additionalProperties".into(), json!(false));
    Arc::new(map)
}

/// Returns the MCP gateway tool definitions.
pub fn mcp_gateway_tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "list-mcp-tools".into(),
            title: Some("List MCP tools with minimal context".into()),
            description: Some(
                "List available MCP tools with minimal context cost. Returns tool names, \
                 descriptions, and estimated token costs without loading full schemas. \
                 Use this to discover tools before loading their full definitions."
                    .into(),
            ),
            input_schema: schema(json!({
                "source": {
                    "type": "string",
                    "description": "Filter by source server (e.g., 'playwright', 'notion')"
                },
                "category": {
                    "type": "string",
                    "description": "Filter by category (e.g., 'browser', 'database')"
                },
                "search": {
                    "type": "string",
                    "description": "Search term to filter tool names and descriptions"
                }
            })),
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
        Tool {
            name: "describe-mcp-tool".into(),
            title: Some("Get full schema for an MCP tool".into()),
            description: Some(
                "Get the full JSON schema for a specific MCP tool. Use this when you need \
                 to invoke a tool and want to see its complete parameter specification. \
                 Only loads the schema for the requested tool, preserving context."
                    .into(),
            ),
            input_schema: schema(json!({
                "tool_name": {
                    "type": "string",
                    "description": "Name of the tool to describe"
                }
            })),
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
        Tool {
            name: "get-context-stats".into(),
            title: Some("View context usage statistics".into()),
            description: Some(
                "Get context usage statistics for MCP tools. Shows tokens saved by lazy loading, \
                 number of schemas loaded, and tool invocation patterns."
                    .into(),
            ),
            input_schema: {
                let mut map = JsonMap::new();
                map.insert("type".into(), json!("object"));
                map.insert("properties".into(), json!({}));
                map.insert("additionalProperties".into(), json!(false));
                Arc::new(map)
            },
            output_schema: None,
            annotations: Some(ToolAnnotations::default()),
            icons: None,
            meta: None,
        },
    ]
}

/// Handle list-mcp-tools request.
pub fn list_mcp_tools(
    args: Option<&JsonMap<String, Value>>,
    entries: Vec<&McpToolEntry>,
) -> Result<CallToolResult> {
    let source_filter = args.and_then(|a| a.get("source")).and_then(|v| v.as_str());
    let category_filter = args
        .and_then(|a| a.get("category"))
        .and_then(|v| v.as_str());
    let search_term = args
        .and_then(|a| a.get("search"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_lowercase());

    let filtered: Vec<_> = entries
        .into_iter()
        .filter(|e| {
            if let Some(src) = source_filter {
                if !e.source.eq_ignore_ascii_case(src) {
                    return false;
                }
            }
            if let Some(cat) = category_filter {
                if e.category
                    .as_ref()
                    .is_none_or(|c| !c.eq_ignore_ascii_case(cat))
                {
                    return false;
                }
            }
            if let Some(ref term) = search_term {
                let name_match = e.name.to_lowercase().contains(term);
                let desc_match = e.description.to_lowercase().contains(term);
                if !name_match && !desc_match {
                    return false;
                }
            }
            true
        })
        .collect();

    let total_tokens: usize = filtered.iter().map(|e| e.estimated_tokens).sum();
    let tool_list: Vec<_> = filtered
        .iter()
        .map(|e| {
            json!({
                "name": e.name,
                "description": e.description,
                "source": e.source,
                "category": e.category,
                "estimated_tokens": e.estimated_tokens
            })
        })
        .collect();

    let result = json!({
        "tools": tool_list,
        "count": tool_list.len(),
        "total_estimated_tokens": total_tokens,
        "hint": "Use 'describe-mcp-tool' to load full schema for a specific tool"
    });

    Ok(CallToolResult {
        content: vec![Content::text(serde_json::to_string_pretty(&result)?)],
        is_error: Some(false),
        structured_content: Some(result),
        meta: None,
    })
}

/// Handle describe-mcp-tool request.
pub fn describe_mcp_tool(
    args: Option<&JsonMap<String, Value>>,
    tool_finder: impl Fn(&str) -> Option<Tool>,
) -> Result<CallToolResult> {
    let tool_name = args
        .and_then(|a| a.get("tool_name"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("tool_name is required"))?;

    match tool_finder(tool_name) {
        Some(tool) => {
            let schema = serde_json::to_value(&tool.input_schema)?;
            let result = json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": schema,
                "loaded": true
            });

            Ok(CallToolResult {
                content: vec![Content::text(serde_json::to_string_pretty(&result)?)],
                is_error: Some(false),
                structured_content: Some(result),
                meta: None,
            })
        }
        None => {
            let result = json!({
                "error": format!("Tool '{}' not found", tool_name),
                "hint": "Use 'list-mcp-tools' to see available tools"
            });

            Ok(CallToolResult {
                content: vec![Content::text(serde_json::to_string_pretty(&result)?)],
                is_error: Some(true),
                structured_content: Some(result),
                meta: None,
            })
        }
    }
}

/// Handle get-context-stats request.
pub fn get_context_stats(stats: ContextStatsSnapshot) -> Result<CallToolResult> {
    let efficiency = if stats.schemas_loaded == 0 {
        "∞ (no schemas loaded yet)".to_string()
    } else {
        format!("{:.1}x", stats.efficiency_ratio())
    };

    let result = json!({
        "tokens_saved": stats.tokens_saved,
        "schemas_loaded": stats.schemas_loaded,
        "total_invocations": stats.total_invocations,
        "efficiency": efficiency,
        "category_tokens": stats.category_tokens,
        "summary": format!(
            "Saved ~{} tokens. {} schemas loaded for {} invocations.",
            stats.tokens_saved, stats.schemas_loaded, stats.total_invocations
        )
    });

    Ok(CallToolResult {
        content: vec![Content::text(serde_json::to_string_pretty(&result)?)],
        is_error: Some(false),
        structured_content: Some(result),
        meta: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_mcp_tools_filtering() {
        let entries = [
            McpToolEntry {
                name: "browser_snapshot".into(),
                description: "Take screenshot".into(),
                source: "playwright".into(),
                estimated_tokens: 150,
                category: Some("browser".into()),
            },
            McpToolEntry {
                name: "notion_search".into(),
                description: "Search Notion".into(),
                source: "notion".into(),
                estimated_tokens: 300,
                category: Some("database".into()),
            },
        ];
        let entry_refs: Vec<_> = entries.iter().collect();

        // No filter
        let result = list_mcp_tools(None, entry_refs.clone()).unwrap();
        let json: Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["count"], 2);

        // Source filter
        let mut args = JsonMap::new();
        args.insert("source".into(), json!("playwright"));
        let result = list_mcp_tools(Some(&args), entry_refs.clone()).unwrap();
        let json: Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["count"], 1);

        // Search filter
        let mut args = JsonMap::new();
        args.insert("search".into(), json!("snapshot"));
        let result = list_mcp_tools(Some(&args), entry_refs).unwrap();
        let json: Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["count"], 1);
    }

    #[test]
    fn test_describe_mcp_tool() {
        let finder = |name: &str| {
            if name == "test_tool" {
                Some(Tool {
                    name: "test_tool".into(),
                    title: Some("Test Tool".into()),
                    description: Some("A test tool".into()),
                    input_schema: {
                        let mut map = JsonMap::new();
                        map.insert("type".into(), json!("object"));
                        Arc::new(map)
                    },
                    output_schema: None,
                    annotations: None,
                    icons: None,
                    meta: None,
                })
            } else {
                None
            }
        };

        // Found
        let mut args = JsonMap::new();
        args.insert("tool_name".into(), json!("test_tool"));
        let result = describe_mcp_tool(Some(&args), finder).unwrap();
        assert_eq!(result.is_error, Some(false));

        // Not found
        let mut args = JsonMap::new();
        args.insert("tool_name".into(), json!("unknown_tool"));
        let result = describe_mcp_tool(Some(&args), finder).unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn test_get_context_stats() {
        let stats = ContextStatsSnapshot {
            tokens_saved: 5000,
            schemas_loaded: 3,
            total_invocations: 10,
            category_tokens: std::collections::HashMap::new(),
        };

        let result = get_context_stats(stats).unwrap();
        assert_eq!(result.is_error, Some(false));
        let json: Value =
            serde_json::from_str(result.content[0].as_text().unwrap().text.as_str()).unwrap();
        assert_eq!(json["tokens_saved"], 5000);
    }

    #[test]
    fn test_mcp_gateway_tools_have_required_fields() {
        let tools = mcp_gateway_tools();
        assert_eq!(tools.len(), 3, "Expected 3 gateway tools");

        for tool in &tools {
            assert!(!tool.name.is_empty(), "Tool name should not be empty");
            assert!(tool.description.is_some(), "Tool should have description");
            assert!(tool.annotations.is_some(), "Tool should have annotations");
        }

        // Verify specific tools exist
        let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();
        assert!(
            names.contains(&"list-mcp-tools"),
            "Should have list-mcp-tools"
        );
        assert!(
            names.contains(&"describe-mcp-tool"),
            "Should have describe-mcp-tool"
        );
        assert!(
            names.contains(&"get-context-stats"),
            "Should have get-context-stats"
        );
    }
}
