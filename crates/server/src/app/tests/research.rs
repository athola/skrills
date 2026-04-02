//! Research tool tests - Knowledge graph, citation tracking, TRIZ resolution
//!
//! Tests the 5 sync research tool handlers that operate on local databases,
//! plus parameter validation for all 9 tools.

use super::super::*;
use serde_json::json;
use std::time::Duration;

// -------------------------------------------------------------------------
// resolve_contradiction_tool Tests
// -------------------------------------------------------------------------

/// GIVEN valid TRIZ parameters
/// WHEN resolve_contradiction_tool is called
/// THEN it returns applicable principles
#[test]
fn resolve_contradiction_returns_principles() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "improve": "performance",
        "degrades": "reliability"
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.resolve_contradiction_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    let principles = structured
        .get("principles")
        .and_then(|v| v.as_array())
        .unwrap();
    assert!(
        !principles.is_empty(),
        "Should return at least one principle"
    );

    // Each principle should have number, name, description
    let first = &principles[0];
    assert!(first.get("number").is_some());
    assert!(first.get("name").is_some());
    assert!(first.get("description").is_some());
}

/// GIVEN an unknown TRIZ parameter
/// WHEN resolve_contradiction_tool is called
/// THEN it returns an error
#[test]
fn resolve_contradiction_rejects_unknown_parameter() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "improve": "speed_of_light",
        "degrades": "reliability"
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.resolve_contradiction_tool(args);
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Unknown parameter"),
        "Should mention unknown parameter"
    );
}

/// GIVEN missing required parameters
/// WHEN resolve_contradiction_tool is called
/// THEN it returns an error for each missing param
#[test]
fn resolve_contradiction_requires_both_params() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Missing 'degrades'
    let args = json!({"improve": "performance"})
        .as_object()
        .cloned()
        .unwrap();
    let err = service.resolve_contradiction_tool(args).unwrap_err();
    assert!(err.to_string().contains("degrades"));

    // Missing 'improve'
    let args = json!({"degrades": "reliability"})
        .as_object()
        .cloned()
        .unwrap();
    let err = service.resolve_contradiction_tool(args).unwrap_err();
    assert!(err.to_string().contains("improve"));
}

/// GIVEN all 15 valid TRIZ parameters
/// WHEN each is used as 'improve'
/// THEN parsing succeeds (no error)
#[test]
fn resolve_contradiction_accepts_all_parameters() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let all_params = [
        "performance",
        "reliability",
        "maintainability",
        "scalability",
        "security",
        "usability",
        "testability",
        "deployability",
        "cost_efficiency",
        "development_speed",
        "code_complexity",
        "memory_usage",
        "latency",
        "throughput",
        "availability",
    ];

    for param in all_params {
        let args = json!({
            "improve": param,
            "degrades": "reliability"
        })
        .as_object()
        .cloned()
        .unwrap();

        let result = service.resolve_contradiction_tool(args);
        assert!(
            result.is_ok(),
            "Parameter '{}' should be accepted, got: {:?}",
            param,
            result.err()
        );
    }
}

// -------------------------------------------------------------------------
// Knowledge Graph Tool Tests
// -------------------------------------------------------------------------

/// GIVEN a fresh knowledge graph database
/// WHEN add_knowledge_node_tool is called with valid args
/// THEN the node is created and can be queried
#[test]
fn add_and_query_knowledge_node() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Add a node
    let args = json!({
        "id": "topic-rust-async",
        "kind": "topic",
        "label": "Rust Async Programming"
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.add_knowledge_node_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    assert_eq!(structured.get("id").unwrap(), "topic-rust-async");
    assert_eq!(structured.get("kind").unwrap(), "topic");

    // Query the node by ID
    let args = json!({"node_id": "topic-rust-async"})
        .as_object()
        .cloned()
        .unwrap();

    let result = service.query_knowledge_graph_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    let node = structured.get("node").unwrap();
    assert_eq!(node.get("label").unwrap(), "Rust Async Programming");
}

/// GIVEN missing required parameters
/// WHEN add_knowledge_node_tool is called
/// THEN it returns appropriate errors
#[test]
fn add_knowledge_node_requires_params() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Missing 'id'
    let args = json!({"kind": "topic", "label": "test"})
        .as_object()
        .cloned()
        .unwrap();
    let err = service.add_knowledge_node_tool(args).unwrap_err();
    assert!(err.to_string().contains("id"));

    // Missing 'kind'
    let args = json!({"id": "test", "label": "test"})
        .as_object()
        .cloned()
        .unwrap();
    let err = service.add_knowledge_node_tool(args).unwrap_err();
    assert!(err.to_string().contains("kind"));

    // Missing 'label'
    let args = json!({"id": "test", "kind": "topic"})
        .as_object()
        .cloned()
        .unwrap();
    let err = service.add_knowledge_node_tool(args).unwrap_err();
    assert!(err.to_string().contains("label"));
}

/// GIVEN an unknown node kind
/// WHEN add_knowledge_node_tool is called
/// THEN it returns an error
#[test]
fn add_knowledge_node_rejects_unknown_kind() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "id": "test-node",
        "kind": "banana",
        "label": "Invalid kind"
    })
    .as_object()
    .cloned()
    .unwrap();

    let err = service.add_knowledge_node_tool(args).unwrap_err();
    assert!(err.to_string().contains("Unknown node kind"));
}

/// GIVEN an empty knowledge graph
/// WHEN query_knowledge_graph_tool is called with no args
/// THEN it returns stats with zero counts
#[test]
fn query_knowledge_graph_returns_stats() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({}).as_object().cloned().unwrap();
    let result = service.query_knowledge_graph_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    assert_eq!(structured.get("node_count").unwrap(), 0);
    assert_eq!(structured.get("edge_count").unwrap(), 0);
}

// -------------------------------------------------------------------------
// link_knowledge_tool Tests
// -------------------------------------------------------------------------

/// GIVEN two nodes in the knowledge graph
/// WHEN link_knowledge_tool is called
/// THEN an edge is created between them
#[test]
fn link_knowledge_creates_edge() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Add two nodes first
    let add = |id: &str, kind: &str, label: &str| {
        let args = json!({"id": id, "kind": kind, "label": label})
            .as_object()
            .cloned()
            .unwrap();
        service.add_knowledge_node_tool(args).unwrap();
    };
    add("paper-1", "paper", "Async Rust Survey");
    add("topic-async", "topic", "Async Programming");

    // Link them
    let args = json!({
        "source_id": "paper-1",
        "target_id": "topic-async",
        "kind": "implements",
        "weight": 0.9
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.link_knowledge_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    assert_eq!(structured.get("source_id").unwrap(), "paper-1");
    assert_eq!(structured.get("target_id").unwrap(), "topic-async");
    assert_eq!(structured.get("kind").unwrap(), "implements");

    // Verify edge exists via query
    let args = json!({"node_id": "paper-1", "direction": "from"})
        .as_object()
        .cloned()
        .unwrap();
    let result = service.query_knowledge_graph_tool(args).unwrap();
    let structured = result.structured_content.unwrap();
    let edges = structured.get("edges_from").unwrap().as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].get("target").unwrap(), "topic-async");
}

/// GIVEN missing required parameters
/// WHEN link_knowledge_tool is called
/// THEN it returns errors
#[test]
fn link_knowledge_requires_params() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Missing source_id
    let args = json!({"target_id": "t", "kind": "cites"})
        .as_object()
        .cloned()
        .unwrap();
    assert!(service
        .link_knowledge_tool(args)
        .unwrap_err()
        .to_string()
        .contains("source_id"));

    // Missing target_id
    let args = json!({"source_id": "s", "kind": "cites"})
        .as_object()
        .cloned()
        .unwrap();
    assert!(service
        .link_knowledge_tool(args)
        .unwrap_err()
        .to_string()
        .contains("target_id"));

    // Missing kind
    let args = json!({"source_id": "s", "target_id": "t"})
        .as_object()
        .cloned()
        .unwrap();
    assert!(service
        .link_knowledge_tool(args)
        .unwrap_err()
        .to_string()
        .contains("kind"));
}

/// GIVEN an unknown edge kind
/// WHEN link_knowledge_tool is called
/// THEN it returns an error
#[test]
fn link_knowledge_rejects_unknown_edge_kind() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "source_id": "a",
        "target_id": "b",
        "kind": "banana"
    })
    .as_object()
    .cloned()
    .unwrap();

    let err = service.link_knowledge_tool(args).unwrap_err();
    assert!(err.to_string().contains("Unknown edge kind"));
}

// -------------------------------------------------------------------------
// track_citations_tool Tests
// -------------------------------------------------------------------------

/// GIVEN a fresh citation database
/// WHEN track_citations_tool is called with action=track
/// THEN the paper is tracked
#[test]
fn track_citations_tracks_paper() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "paper_id": "paper-001",
        "action": "track",
        "title": "Async Rust Patterns",
        "doi": "10.1234/async-rust"
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.track_citations_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    assert_eq!(structured.get("action").unwrap(), "tracked");
    assert_eq!(structured.get("paper_id").unwrap(), "paper-001");
}

/// GIVEN a tracked paper with no citations
/// WHEN track_citations_tool is called with action=forward
/// THEN it returns an empty citations list
#[test]
fn track_citations_forward_empty() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Track a paper first
    let args = json!({
        "paper_id": "paper-002",
        "action": "track",
        "title": "Test Paper"
    })
    .as_object()
    .cloned()
    .unwrap();
    service.track_citations_tool(args).unwrap();

    // Query forward citations
    let args = json!({
        "paper_id": "paper-002",
        "action": "forward"
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.track_citations_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    assert_eq!(structured.get("count").unwrap(), 0);
    assert_eq!(structured.get("direction").unwrap(), "forward");
}

/// GIVEN missing required parameters
/// WHEN track_citations_tool is called
/// THEN it returns errors
#[test]
fn track_citations_requires_paper_id() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"action": "track", "title": "No ID"})
        .as_object()
        .cloned()
        .unwrap();
    let err = service.track_citations_tool(args).unwrap_err();
    assert!(err.to_string().contains("paper_id"));
}

/// GIVEN a track action without title
/// WHEN track_citations_tool is called
/// THEN it returns an error about missing title
#[test]
fn track_citations_track_requires_title() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "paper_id": "paper-003",
        "action": "track"
    })
    .as_object()
    .cloned()
    .unwrap();

    let err = service.track_citations_tool(args).unwrap_err();
    assert!(err.to_string().contains("title"));
}

/// GIVEN an unknown action
/// WHEN track_citations_tool is called
/// THEN it returns an error
#[test]
fn track_citations_rejects_unknown_action() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "paper_id": "paper-004",
        "action": "explode"
    })
    .as_object()
    .cloned()
    .unwrap();

    let err = service.track_citations_tool(args).unwrap_err();
    assert!(err.to_string().contains("Unknown action"));
}

// -------------------------------------------------------------------------
// Tool Schema Tests
// -------------------------------------------------------------------------

/// GIVEN the research tool schemas
/// WHEN inspecting required fields
/// THEN each tool has correct required parameters
#[test]
fn research_tool_schemas_have_correct_required_fields() {
    use crate::tool_schemas::research_tools;

    let tools = research_tools();
    let tool_map: std::collections::HashMap<&str, &rmcp::model::Tool> =
        tools.iter().map(|t| (t.name.as_ref(), t)).collect();

    // search-papers requires "query"
    let schema = tool_map["search-papers"].input_schema.as_ref();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("query")));

    // search-discussions requires "query"
    let schema = tool_map["search-discussions"].input_schema.as_ref();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("query")));

    // resolve-doi requires "doi"
    let schema = tool_map["resolve-doi"].input_schema.as_ref();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("doi")));

    // fetch-pdf requires "doi"
    let schema = tool_map["fetch-pdf"].input_schema.as_ref();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("doi")));

    // add-knowledge-node requires "id", "kind", "label"
    let schema = tool_map["add-knowledge-node"].input_schema.as_ref();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("id")));
    assert!(required.contains(&json!("kind")));
    assert!(required.contains(&json!("label")));

    // link-knowledge requires "source_id", "target_id", "kind"
    let schema = tool_map["link-knowledge"].input_schema.as_ref();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("source_id")));
    assert!(required.contains(&json!("target_id")));
    assert!(required.contains(&json!("kind")));

    // resolve-contradiction requires "improve", "degrades"
    let schema = tool_map["resolve-contradiction"].input_schema.as_ref();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("improve")));
    assert!(required.contains(&json!("degrades")));
}

/// GIVEN the research tool schemas
/// WHEN checking tool names
/// THEN all 9 expected tools are present
#[test]
fn research_tools_has_all_expected_names() {
    use crate::tool_schemas::research_tools;

    let tools = research_tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();

    let expected = [
        "search-papers",
        "search-discussions",
        "resolve-doi",
        "fetch-pdf",
        "query-knowledge-graph",
        "add-knowledge-node",
        "link-knowledge",
        "track-citations",
        "resolve-contradiction",
    ];

    for name in expected {
        assert!(
            names.contains(&name),
            "Expected tool '{}' in research_tools, found: {:?}",
            name,
            names
        );
    }
}
