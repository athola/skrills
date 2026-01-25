//! MCP tests - MCP registry and tool operations

use super::super::*;
use serde_json::json;

#[test]
fn test_mcp_registry_is_populated_on_service_creation() {
    // Build MCP registry and verify it has tools registered
    let registry = build_mcp_registry();

    // Should have at least the 22 internal tools + 3 gateway tools
    assert!(
        registry.len() >= 25,
        "Expected at least 25 tools, got {}",
        registry.len()
    );

    // Verify source categories exist
    let sources = registry.sources();
    assert!(
        sources.contains(&"skrills"),
        "Registry should have skrills tools"
    );
    assert!(
        sources.contains(&"gateway"),
        "Registry should have gateway tools"
    );

    // Verify we can look up specific tools
    assert!(
        registry.get("sync-skills").is_some(),
        "Should find sync-skills tool"
    );
    assert!(
        registry.get("list-mcp-tools").is_some(),
        "Should find list-mcp-tools gateway tool"
    );

    // Verify token estimates are reasonable
    let total_tokens = registry.total_estimated_tokens();
    assert!(
        total_tokens > 100 && total_tokens < 100_000,
        "Total tokens {} should be in reasonable range",
        total_tokens
    );
}

// -------------------------------------------------------------------------
// MCP Gateway Tool Handler Tests
// -------------------------------------------------------------------------

/// GIVEN an MCP gateway with registered tools
/// WHEN list-mcp-tools is called without filters
/// THEN it should return all tools with their metadata
#[test]
fn test_list_mcp_tools_returns_all_tools() {
    use crate::mcp_gateway::{list_mcp_tools, McpToolEntry};

    let entries = [
        McpToolEntry {
            name: "test-tool".into(),
            description: "A test tool".into(),
            source: "skrills".into(),
            estimated_tokens: 100,
            category: Some("testing".into()),
        },
        McpToolEntry {
            name: "another-tool".into(),
            description: "Another tool".into(),
            source: "gateway".into(),
            estimated_tokens: 50,
            category: None,
        },
    ];
    let entry_refs: Vec<_> = entries.iter().collect();

    let result = list_mcp_tools(None, entry_refs).unwrap();

    // Verify structured content
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["count"], 2);
    assert_eq!(structured["total_estimated_tokens"], 150);

    let tools = structured["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
}

/// GIVEN an MCP gateway with tools from multiple sources
/// WHEN list-mcp-tools is called with source filter
/// THEN it should return only tools from that source
#[test]
fn test_list_mcp_tools_filters_by_source() {
    use crate::mcp_gateway::{list_mcp_tools, McpToolEntry};

    let entries = [
        McpToolEntry {
            name: "skrills-tool".into(),
            description: "From skrills".into(),
            source: "skrills".into(),
            estimated_tokens: 100,
            category: None,
        },
        McpToolEntry {
            name: "gateway-tool".into(),
            description: "From gateway".into(),
            source: "gateway".into(),
            estimated_tokens: 50,
            category: None,
        },
    ];
    let entry_refs: Vec<_> = entries.iter().collect();

    let mut args = serde_json::Map::new();
    args.insert("source".into(), json!("gateway"));

    let result = list_mcp_tools(Some(&args), entry_refs).unwrap();
    let structured = result.structured_content.unwrap();

    assert_eq!(structured["count"], 1);
    let tools = structured["tools"].as_array().unwrap();
    assert_eq!(tools[0]["name"], "gateway-tool");
}

/// GIVEN an MCP gateway
/// WHEN describe-mcp-tool is called with a valid tool name
/// THEN it should return the full tool schema
#[test]
fn test_describe_mcp_tool_returns_schema() {
    use crate::mcp_gateway::describe_mcp_tool;
    use rmcp::model::Tool;
    use std::sync::Arc;

    let finder = |name: &str| -> Option<Tool> {
        if name == "sync-skills" {
            Some(Tool {
                name: "sync-skills".into(),
                title: Some("Sync Skills".into()),
                description: Some("Synchronize skills".into()),
                input_schema: {
                    let mut map = serde_json::Map::new();
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

    let mut args = serde_json::Map::new();
    args.insert("tool_name".into(), json!("sync-skills"));

    let result = describe_mcp_tool(Some(&args), finder).unwrap();

    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["name"], "sync-skills");
    assert_eq!(structured["loaded"], true);
}

/// GIVEN an MCP gateway
/// WHEN describe-mcp-tool is called with an unknown tool name
/// THEN it should return an error result
#[test]
fn test_describe_mcp_tool_unknown_returns_error() {
    use crate::mcp_gateway::describe_mcp_tool;

    let finder = |_: &str| None;

    let mut args = serde_json::Map::new();
    args.insert("tool_name".into(), json!("nonexistent-tool"));

    let result = describe_mcp_tool(Some(&args), finder).unwrap();

    assert_eq!(result.is_error, Some(true));
    let structured = result.structured_content.unwrap();
    assert!(structured["error"].as_str().unwrap().contains("not found"));
}

/// GIVEN a ContextStats tracker
/// WHEN get-context-stats is called
/// THEN it should return current statistics
#[test]
fn test_get_context_stats_returns_snapshot() {
    use crate::mcp_gateway::{get_context_stats, ContextStatsSnapshot};
    use std::collections::HashMap;

    let stats = ContextStatsSnapshot {
        tokens_saved: 1000,
        schemas_loaded: 5,
        total_invocations: 25,
        category_tokens: HashMap::from([("browser".to_string(), 500)]),
    };

    let result = get_context_stats(stats).unwrap();

    assert_eq!(result.is_error, Some(false));
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["tokens_saved"], 1000);
    assert_eq!(structured["schemas_loaded"], 5);
    assert_eq!(structured["total_invocations"], 25);
    // efficiency = 25 / 5 = 5.0x
    assert_eq!(structured["efficiency"], "5.0x");
}

/// GIVEN a ContextStats tracker with no schemas loaded
/// WHEN get-context-stats is called
/// THEN efficiency should show infinity
#[test]
fn test_get_context_stats_infinity_efficiency() {
    use crate::mcp_gateway::{get_context_stats, ContextStatsSnapshot};
    use std::collections::HashMap;

    let stats = ContextStatsSnapshot {
        tokens_saved: 500,
        schemas_loaded: 0,
        total_invocations: 10,
        category_tokens: HashMap::new(),
    };

    let result = get_context_stats(stats).unwrap();

    let structured = result.structured_content.unwrap();
    assert!(structured["efficiency"].as_str().unwrap().contains("∞"));
}
