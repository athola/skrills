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

    // track-citations requires only "paper_id" (T4: title is optional, only needed for track action)
    let schema = tool_map["track-citations"].input_schema.as_ref();
    let required = schema.get("required").unwrap().as_array().unwrap();
    assert!(required.contains(&json!("paper_id")));
    assert!(
        !required.contains(&json!("title")),
        "title should not be in required; only needed for action=track"
    );

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

// -------------------------------------------------------------------------
// T1: query_knowledge_graph_tool search-by-query branch
// -------------------------------------------------------------------------

/// GIVEN nodes in the knowledge graph
/// WHEN query_knowledge_graph_tool is called with a query string
/// THEN it returns matching nodes
#[test]
fn query_knowledge_graph_search_by_query() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Add a node
    let args = json!({"id": "topic-search-test", "kind": "topic", "label": "Searchable Topic"})
        .as_object()
        .cloned()
        .unwrap();
    service.add_knowledge_node_tool(args).unwrap();

    // Search by query
    let args = json!({"query": "Searchable"}).as_object().cloned().unwrap();
    let result = service.query_knowledge_graph_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    let nodes = structured.get("nodes").unwrap().as_array().unwrap();
    assert!(!nodes.is_empty(), "Should find at least one node");
    assert_eq!(nodes[0].get("id").unwrap(), "topic-search-test");
}

/// GIVEN nodes in the knowledge graph
/// WHEN query_knowledge_graph_tool is called with query and kind filter
/// THEN it filters by kind
#[test]
fn query_knowledge_graph_search_with_kind_filter() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"id": "paper-filter", "kind": "paper", "label": "Filter Test Paper"})
        .as_object()
        .cloned()
        .unwrap();
    service.add_knowledge_node_tool(args).unwrap();

    let args = json!({"id": "topic-filter", "kind": "topic", "label": "Filter Test Topic"})
        .as_object()
        .cloned()
        .unwrap();
    service.add_knowledge_node_tool(args).unwrap();

    // Search with kind=paper filter
    let args = json!({"query": "Filter", "kind": "paper"})
        .as_object()
        .cloned()
        .unwrap();
    let result = service.query_knowledge_graph_tool(args).unwrap();
    let structured = result.structured_content.unwrap();
    let nodes = structured.get("nodes").unwrap().as_array().unwrap();

    for node in nodes {
        assert_eq!(node.get("kind").unwrap(), "paper");
    }
}

/// GIVEN a query with an unknown kind
/// WHEN query_knowledge_graph_tool is called
/// THEN it returns an error (I3)
#[test]
fn query_knowledge_graph_rejects_unknown_kind() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"query": "anything", "kind": "banana"})
        .as_object()
        .cloned()
        .unwrap();
    let err = service.query_knowledge_graph_tool(args).unwrap_err();
    assert!(
        err.to_string().contains("Unknown node kind"),
        "Should reject unknown kind, got: {err}"
    );
}

// -------------------------------------------------------------------------
// T3: track_citations_tool backward action
// -------------------------------------------------------------------------

/// GIVEN a tracked paper
/// WHEN track_citations_tool is called with action=backward
/// THEN it returns backward citations (empty for new paper)
#[test]
fn track_citations_backward_empty() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Track a paper first
    let args = json!({
        "paper_id": "paper-back-001",
        "action": "track",
        "title": "Backward Test Paper"
    })
    .as_object()
    .cloned()
    .unwrap();
    service.track_citations_tool(args).unwrap();

    // Query backward citations
    let args = json!({
        "paper_id": "paper-back-001",
        "action": "backward"
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.track_citations_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    let structured = result.structured_content.unwrap();
    assert_eq!(structured.get("count").unwrap(), 0);
    assert_eq!(structured.get("direction").unwrap(), "backward");
}

// -------------------------------------------------------------------------
// T5: More edge kinds and direction filters
// -------------------------------------------------------------------------

/// GIVEN two nodes
/// WHEN linked with each of the 5 edge kinds
/// THEN all are accepted
#[test]
fn link_knowledge_accepts_all_edge_kinds() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Add source and target nodes
    let add = |id: &str| {
        let args = json!({"id": id, "kind": "topic", "label": id})
            .as_object()
            .cloned()
            .unwrap();
        service.add_knowledge_node_tool(args).unwrap();
    };
    add("src-node");
    add("tgt-cites");
    add("tgt-impl");
    add("tgt-contra");
    add("tgt-ext");
    add("tgt-analog");

    let all_kinds = [
        "cites",
        "implements",
        "contradicts",
        "extends",
        "analogous_to",
    ];
    let targets = [
        "tgt-cites",
        "tgt-impl",
        "tgt-contra",
        "tgt-ext",
        "tgt-analog",
    ];

    for (kind, target) in all_kinds.iter().zip(targets.iter()) {
        let args = json!({
            "source_id": "src-node",
            "target_id": target,
            "kind": kind,
        })
        .as_object()
        .cloned()
        .unwrap();

        let result = service.link_knowledge_tool(args);
        assert!(
            result.is_ok(),
            "Edge kind '{}' should be accepted, got: {:?}",
            kind,
            result.err()
        );
    }
}

/// GIVEN a node with both incoming and outgoing edges
/// WHEN query_knowledge_graph_tool is called with direction="to"
/// THEN only incoming edges are returned
#[test]
fn query_knowledge_graph_direction_to() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let add = |id: &str| {
        let args = json!({"id": id, "kind": "topic", "label": id})
            .as_object()
            .cloned()
            .unwrap();
        service.add_knowledge_node_tool(args).unwrap();
    };
    add("center");
    add("outgoing-target");
    add("incoming-source");

    // center -> outgoing-target
    let args = json!({"source_id": "center", "target_id": "outgoing-target", "kind": "cites"})
        .as_object()
        .cloned()
        .unwrap();
    service.link_knowledge_tool(args).unwrap();

    // incoming-source -> center
    let args = json!({"source_id": "incoming-source", "target_id": "center", "kind": "extends"})
        .as_object()
        .cloned()
        .unwrap();
    service.link_knowledge_tool(args).unwrap();

    // Query with direction="to", should get only incoming edges
    let args = json!({"node_id": "center", "direction": "to"})
        .as_object()
        .cloned()
        .unwrap();
    let result = service.query_knowledge_graph_tool(args).unwrap();
    let structured = result.structured_content.unwrap();

    let edges_from = structured.get("edges_from").unwrap().as_array().unwrap();
    let edges_to = structured.get("edges_to").unwrap().as_array().unwrap();
    assert!(
        edges_from.is_empty(),
        "direction=to should have no outgoing edges"
    );
    assert_eq!(edges_to.len(), 1);
    assert_eq!(edges_to[0].get("source").unwrap(), "incoming-source");
}

// -------------------------------------------------------------------------
// Enum sync test, verify TRIZ parameter, NodeKind, and EdgeKind
//     enums are consistent between schema and handler
// -------------------------------------------------------------------------

/// GIVEN the resolve-contradiction schema enum values
/// WHEN compared against Parameter::all() (canonical source)
/// THEN they match exactly
#[test]
fn triz_parameter_enum_matches_schema() {
    use crate::tool_schemas::research_tools;
    use skrills_tome::triz::Parameter;

    let tools = research_tools();
    let tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "resolve-contradiction")
        .unwrap();

    let schema = tool.input_schema.as_ref();
    let props = schema.get("properties").unwrap();
    let improve_enum = props
        .get("improve")
        .unwrap()
        .get("enum")
        .unwrap()
        .as_array()
        .unwrap();

    let schema_values: Vec<&str> = improve_enum.iter().filter_map(|v| v.as_str()).collect();
    let enum_values: Vec<&str> = Parameter::all().iter().map(|p| p.as_str()).collect();

    assert_eq!(
        schema_values.len(),
        enum_values.len(),
        "Schema enum count ({}) != Parameter::all() count ({})",
        schema_values.len(),
        enum_values.len()
    );
    for val in &enum_values {
        assert!(
            schema_values.contains(val),
            "Parameter::all() has '{}' but schema enum does not include it",
            val
        );
    }
}

/// GIVEN the add-knowledge-node schema enum values
/// WHEN compared against NodeKind::all() (canonical source)
/// THEN they match exactly
#[test]
fn node_kind_enum_matches_schema() {
    use crate::tool_schemas::research_tools;
    use skrills_tome::knowledge_graph::NodeKind;

    let tools = research_tools();
    let tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "add-knowledge-node")
        .unwrap();

    let schema = tool.input_schema.as_ref();
    let kind_enum = schema
        .get("properties")
        .unwrap()
        .get("kind")
        .unwrap()
        .get("enum")
        .unwrap()
        .as_array()
        .unwrap();

    let schema_values: Vec<&str> = kind_enum.iter().filter_map(|v| v.as_str()).collect();
    let enum_values: Vec<&str> = NodeKind::all().iter().map(|k| k.as_str()).collect();

    assert_eq!(schema_values.len(), enum_values.len());
    for val in &enum_values {
        assert!(
            schema_values.contains(val),
            "NodeKind::all() has '{}' but schema enum does not",
            val
        );
    }
}

/// GIVEN the link-knowledge schema enum values
/// WHEN compared against EdgeKind::all() (canonical source)
/// THEN they match exactly
#[test]
fn edge_kind_enum_matches_schema() {
    use crate::tool_schemas::research_tools;
    use skrills_tome::knowledge_graph::EdgeKind;

    let tools = research_tools();
    let tool = tools
        .iter()
        .find(|t| t.name.as_ref() == "link-knowledge")
        .unwrap();

    let schema = tool.input_schema.as_ref();
    let kind_enum = schema
        .get("properties")
        .unwrap()
        .get("kind")
        .unwrap()
        .get("enum")
        .unwrap()
        .as_array()
        .unwrap();

    let schema_values: Vec<&str> = kind_enum.iter().filter_map(|v| v.as_str()).collect();
    let enum_values: Vec<&str> = EdgeKind::all().iter().map(|k| k.as_str()).collect();

    assert_eq!(schema_values.len(), enum_values.len());
    for val in &enum_values {
        assert!(
            schema_values.contains(val),
            "EdgeKind::all() has '{}' but schema enum does not",
            val
        );
    }
}

// -------------------------------------------------------------------------
// fetch_pdf helpers (SA-8, SA-9, SA-40)
// -------------------------------------------------------------------------

use super::super::research::{check_pdf_url, download_pdf, pdf_cache_file_name};

/// SA-40: DOIs that used to collide get distinct files with no separators.
#[test]
fn pdf_cache_file_name_is_injective_and_flat() {
    let a = pdf_cache_file_name("10.1/a_b");
    let b = pdf_cache_file_name("10.1/a/b");
    assert_ne!(a, b);
    for name in [&a, &b, &pdf_cache_file_name("10.1/../../etc")] {
        assert!(!name.contains('/') && !name.contains('\\'), "{name}");
        assert!(name.ends_with(".pdf"));
    }
}

/// SA-9: only https on a public host is accepted.
#[test]
fn check_pdf_url_rejects_local_and_plain_http_targets() {
    let ok = reqwest::Url::parse("https://arxiv.org/pdf/1234.pdf").unwrap();
    assert!(check_pdf_url(&ok).is_ok());
    for bad in [
        "http://arxiv.org/x.pdf",
        "file:///etc/passwd",
        "https://localhost/x.pdf",
        "https://api.localhost/x.pdf",
        "https://127.0.0.1/x.pdf",
        "https://169.254.169.254/latest/meta-data",
        "https://10.0.0.5/x.pdf",
        "https://192.168.1.1/x.pdf",
        "https://100.64.0.1/x.pdf",
        "https://[::1]/x.pdf",
        "https://[fd00::1]/x.pdf",
        "https://[fe80::1]/x.pdf",
        "https://[::ffff:127.0.0.1]/x.pdf",
    ] {
        let url = reqwest::Url::parse(bad).unwrap();
        assert!(check_pdf_url(&url).is_err(), "{bad} should be refused");
    }
}

async fn serve(body: Vec<u8>) -> wiremock::MockServer {
    use wiremock::matchers::method;
    use wiremock::{Mock, ResponseTemplate};
    let server = wiremock::MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(&server)
        .await;
    server
}

/// SA-8: a PDF is written whole; an HTML error page or an oversized body
/// leaves no cache file behind.
#[tokio::test]
async fn download_pdf_checks_magic_and_size_and_writes_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let client = reqwest::Client::new();

    let pdf = b"%PDF-1.7\nbody".to_vec();
    let server = serve(pdf.clone()).await;
    let dest = dir.path().join("ok.pdf");
    let url = reqwest::Url::parse(&server.uri()).unwrap();
    let n = download_pdf(&client, url, &dest, 1024).await.unwrap();
    assert_eq!(n, pdf.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), pdf);

    let server = serve(b"<html>not found</html>".to_vec()).await;
    let dest = dir.path().join("html.pdf");
    let url = reqwest::Url::parse(&server.uri()).unwrap();
    let err = download_pdf(&client, url, &dest, 1024).await.unwrap_err();
    assert!(err.to_string().contains("did not return a PDF"), "{err}");
    assert!(!dest.exists());

    let mut big = b"%PDF-".to_vec();
    big.resize(4096, b'x');
    let server = serve(big).await;
    let dest = dir.path().join("big.pdf");
    let url = reqwest::Url::parse(&server.uri()).unwrap();
    let err = download_pdf(&client, url, &dest, 1024).await.unwrap_err();
    assert!(err.to_string().contains("larger than"), "{err}");
    assert!(!dest.exists());

    // No stray temporary files are left in the cache directory.
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![std::ffi::OsString::from("ok.pdf")]);
}

/// A cached PDF is served without asking Unpaywall, so it stays reachable
/// offline or while Unpaywall is down. The DOI is not a real one: any call
/// to Unpaywall would fail the request.
#[test]
fn fetch_pdf_serves_a_cached_pdf_without_calling_unpaywall() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let cache_base = temp.path().join("cache");
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));
    let _xdg = crate::test_support::set_env_var("XDG_CACHE_HOME", cache_base.to_str());
    let _local = crate::test_support::set_env_var("LOCALAPPDATA", cache_base.to_str());

    let doi = "10.0000/skrills-cached-only";
    let pdf_dir = skrills_tome::cache::ResearchCache::cache_dir()
        .unwrap()
        .join("pdfs");
    std::fs::create_dir_all(&pdf_dir).unwrap();
    let cached = pdf_dir.join(super::super::research::pdf_cache_file_name(doi));
    std::fs::write(&cached, b"%PDF-1.4 cached").unwrap();

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();
    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(service.fetch_pdf_tool(json!({"doi": doi}).as_object().cloned().unwrap()))
        .expect("a cached PDF needs no network");
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["cached"], true);
    assert_eq!(structured["path"], cached.to_string_lossy().as_ref());
}

/// `["arxiv" x 10000]` made 10 000 outbound calls; sources are deduplicated
/// and unknown names reported without a call.
#[test]
fn search_sources_are_deduplicated_and_unknown_names_reported() {
    let requested: Vec<serde_json::Value> = std::iter::repeat_n(json!("arxiv"), 10_000)
        .chain([json!("openalex"), json!("bogus"), json!("arxiv")])
        .collect();
    let (sources, errors) = super::super::research::paper_sources(Some(&requested));
    assert_eq!(sources, ["arxiv", "openalex"]);
    assert_eq!(errors, ["Unknown source: bogus"]);
    let (all, none) = super::super::research::paper_sources(None);
    assert_eq!(all, ["arxiv", "semantic_scholar", "openalex"]);
    assert!(none.is_empty());
}

/// Results were concatenated in source order and truncated, so with the
/// default limit every result came from arXiv; DOI-less duplicates repeated.
#[test]
fn search_results_interleave_sources_and_drop_duplicate_titles() {
    use skrills_tome::models::{Paper, PaperSource};
    let paper = |title: &str, doi: Option<&str>, source: PaperSource| Paper {
        id: title.into(),
        title: title.into(),
        authors: Vec::new(),
        abstract_text: None,
        year: None,
        doi: doi.map(String::from),
        url: None,
        source,
        citation_count: None,
        pdf_url: None,
    };
    let per_source = vec![
        vec![
            paper("A one", None, PaperSource::Arxiv),
            paper("A two", None, PaperSource::Arxiv),
        ],
        vec![
            paper("a  ONE!", None, PaperSource::OpenAlex),
            paper("B two", Some("10.1/b"), PaperSource::OpenAlex),
        ],
        vec![paper(
            "B again",
            Some("10.1/b"),
            PaperSource::SemanticScholar,
        )],
    ];
    let merged = super::super::research::merge_papers(per_source, 3);
    let titles: Vec<&str> = merged.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, ["A one", "B again", "A two"]);
}

/// Reserved ranges a literal PDF URL could still name.
#[test]
fn check_pdf_url_rejects_reserved_and_embedded_private_addresses() {
    use super::super::research::check_pdf_url;
    for url in [
        "https://0.1.2.3/x.pdf",
        "https://198.18.0.1/x.pdf",
        "https://224.0.0.1/x.pdf",
        "https://240.0.0.1/x.pdf",
        "https://[64:ff9b::a00:1]/x.pdf",
        "https://[::10.0.0.1]/x.pdf",
        "https://[ff02::1]/x.pdf",
        "https://[::ffff:169.254.169.254]/x.pdf",
    ] {
        let parsed = reqwest::Url::parse(url).unwrap();
        assert!(check_pdf_url(&parsed).is_err(), "{url} was accepted");
    }
    let public = reqwest::Url::parse("https://93.184.215.14/x.pdf").unwrap();
    assert!(check_pdf_url(&public).is_ok());
}

/// A public-looking name can resolve to a private address; the download
/// client's resolver refuses such answers.
#[test]
fn pdf_resolver_refuses_names_that_resolve_to_private_addresses() {
    use reqwest::dns::Resolve;
    let resolver = super::super::research::PublicOnlyResolver;
    let outcome = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(resolver.resolve("localhost".parse().unwrap()));
    assert!(outcome.is_err(), "localhost resolved to an allowed address");
}

/// Node fields had no size limit, so a looping client could grow
/// `knowledge.db` without bound.
#[test]
fn add_knowledge_node_refuses_oversized_fields() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let cache_base = temp.path().join("cache");
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));
    let _xdg = crate::test_support::set_env_var("XDG_CACHE_HOME", cache_base.to_str());
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();
    let big = "x".repeat(super::super::research::MAX_KNOWLEDGE_FIELD_BYTES + 1);
    for args in [
        json!({"id": big, "kind": "topic", "label": "l"}),
        json!({"id": "a", "kind": "topic", "label": big}),
        json!({"id": "a", "kind": "topic", "label": "l", "metadata": {"note": big}}),
    ] {
        let err = service
            .add_knowledge_node_tool(args.as_object().cloned().unwrap())
            .expect_err("oversized field accepted");
        assert!(err.to_string().contains("too large"), "{err}");
    }
}

#[test]
fn knowledge_node_budget_admits_updates_but_not_new_nodes_when_full() {
    use super::super::research::{node_budget_allows, MAX_KNOWLEDGE_NODES};
    assert!(node_budget_allows(MAX_KNOWLEDGE_NODES - 1, false));
    assert!(!node_budget_allows(MAX_KNOWLEDGE_NODES, false));
    assert!(node_budget_allows(MAX_KNOWLEDGE_NODES, true));
}

/// Each DOI could add 100 MiB to the PDF cache with no total limit; the
/// oldest files are evicted to keep the cache under its budget.
#[test]
fn pdf_cache_evicts_the_oldest_files_past_its_budget() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path();
    let now = std::time::SystemTime::now();
    for (name, age_secs) in [("old.pdf", 300u64), ("mid.pdf", 200), ("new.pdf", 100)] {
        let path = dir.join(name);
        std::fs::write(&path, vec![b'x'; 10]).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(now - Duration::from_secs(age_secs))
            .unwrap();
    }
    super::super::research::evict_pdf_cache(dir, 20).unwrap();
    assert!(!dir.join("old.pdf").exists());
    assert!(dir.join("mid.pdf").exists() && dir.join("new.pdf").exists());
}

/// Edge metadata had no size limit either; it is refused before the
/// database is opened, so no nodes are needed.
#[test]
fn link_knowledge_refuses_oversized_metadata() {
    let _guard = crate::test_support::env_guard();
    let temp = tempfile::tempdir().unwrap();
    let cache_base = temp.path().join("cache");
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));
    let _xdg = crate::test_support::set_env_var("XDG_CACHE_HOME", cache_base.to_str());
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();
    let big = "x".repeat(super::super::research::MAX_KNOWLEDGE_FIELD_BYTES + 1);
    let args = json!({
        "source_id": "a",
        "target_id": "b",
        "kind": "cites",
        "metadata": {"note": big},
    });
    let err = service
        .link_knowledge_tool(args.as_object().cloned().unwrap())
        .expect_err("oversized metadata accepted");
    assert!(err.to_string().contains("too large"), "{err}");
}

fn run_search_papers(args: serde_json::Value) -> rmcp::model::CallToolResult {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(service.search_papers_tool(args.as_object().cloned().unwrap()))
        .unwrap()
}

fn result_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|t| t.text.clone()))
        .collect()
}

/// Only unknown sources requested: nothing is searched and the result is
/// an error that names every failure.
#[test]
fn search_papers_with_only_unknown_sources_reports_all_sources_failed() {
    let result = run_search_papers(json!({"query": "rust", "sources": ["bogus"]}));
    assert_eq!(result.is_error, Some(true));
    let text = result_text(&result);
    assert_eq!(
        text,
        "Found 0 papers (1 source errors: Unknown source: bogus): all sources failed"
    );
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["errors"], json!(["Unknown source: bogus"]));
    assert_eq!(structured["count"], 0);
}

/// A zero limit makes every client return early, so known sources succeed
/// with no papers and no request; that is not a failure.
#[test]
fn search_papers_with_zero_limit_returns_no_papers_and_no_errors() {
    let result = run_search_papers(json!({
        "query": "rust",
        "limit": 0,
        "sources": ["arxiv", "semantic_scholar", "openalex"],
    }));
    assert_ne!(result.is_error, Some(true));
    assert_eq!(result_text(&result), "Found 0 papers");
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["errors"], json!([]));
    assert_eq!(structured["papers"], json!([]));
}
