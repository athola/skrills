//! MCP tool schema definitions for subagent service.
//!
//! This module defines the JSON schemas for all subagent-related MCP tools.
//! Schemas are separated from the service implementation for maintainability.

use std::sync::Arc;

use rmcp::model::{object, JsonObject, Tool};
use serde_json::json;

/// Generate the run-subagent input schema.
pub fn run_schema() -> Arc<JsonObject> {
    Arc::new(object(json!({
        "type": "object",
        "required": ["prompt"],
        "properties": {
            "prompt": {"type": "string", "description": "User instruction"},
            "agent_id": {"type": "string", "description": "Agent name to run (from list-agents). When specified, routes to appropriate execution path based on agent capabilities."},
            "backend": {"type": "string", "description": "codex|claude|other (used only when execution_mode=api and agent_id is not specified)"},
            "execution_mode": {"type": "string", "description": "cli|api (default: cli). cli uses local headless CLI; api uses network APIs."},
            "cli_binary": {"type": "string", "description": "CLI binary to run in cli mode (overrides SKRILLS_CLI_BINARY/config)"},
            "template_id": {"type": "string"},
            "output_schema": {"type": "object"},
            "tracing": {"type": "boolean"},
            "stream": {"type": "boolean"},
            "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 300000}
        }
    })))
}

/// Generate the run_id input schema.
pub fn run_id_schema() -> Arc<JsonObject> {
    Arc::new(object(json!({
        "type": "object",
        "required": ["run_id"],
        "properties": {"run_id": {"type": "string"}}
    })))
}

/// Generate the history input schema.
pub fn history_schema() -> Arc<JsonObject> {
    Arc::new(object(json!({
        "type": "object",
        "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 50}},
    })))
}

/// Generate the events input schema.
pub fn events_schema() -> Arc<JsonObject> {
    Arc::new(object(json!({
        "type": "object",
        "required": ["run_id"],
        "properties": {
            "run_id": {"type": "string", "description": "The run ID to get events for"},
            "since_index": {"type": "integer", "minimum": 0, "description": "Return events after this index (0-based)"}
        }
    })))
}

/// Generate the events output schema.
pub fn events_output_schema() -> Arc<JsonObject> {
    Arc::new(object(json!({
        "type": "object",
        "properties": {
            "run_id": {"type": "string"},
            "events": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "index": {"type": "integer"},
                        "ts": {"type": "string"},
                        "kind": {"type": "string"},
                        "data": {}
                    }
                }
            },
            "total_count": {"type": "integer"},
            "has_more": {"type": "boolean"}
        }
    })))
}

/// Generate the run output schema.
pub fn run_output_schema() -> Arc<JsonObject> {
    Arc::new(object(json!({
        "type": "object",
        "required": ["run_id"],
        "properties": {
            "run_id": {"type": "string"},
            "status": {"type": "object"},
            "events": {"type": "array", "items": {"type": "object"}}
        }
    })))
}

/// Generate the list output schema.
pub fn list_output_schema() -> Arc<JsonObject> {
    Arc::new(object(json!({
        "type": "object",
        "properties": {"templates": {"type": "array", "items": {"type": "object"}}}
    })))
}

/// Generate the agents output schema.
pub fn agents_output_schema() -> Arc<JsonObject> {
    Arc::new(object(json!({
        "type": "object",
        "properties": {
            "agents": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "description": {"type": "string"},
                        "tools": {"type": "array", "items": {"type": "string"}},
                        "model": {"type": "string"},
                        "source": {"type": "string"},
                        "path": {"type": "string"},
                        "requires_cli": {"type": "boolean"}
                    }
                }
            }
        }
    })))
}

/// Build all subagent tools with their schemas.
pub fn all_tools() -> Vec<Tool> {
    let run_schema = run_schema();
    let run_id_schema = run_id_schema();
    let history_schema = history_schema();
    let events_schema = events_schema();
    let events_output_schema = events_output_schema();
    let run_output_schema = run_output_schema();
    let list_output_schema = list_output_schema();
    let agents_output_schema = agents_output_schema();

    let mut tools = vec![
        Tool::new(
            "list-subagents",
            "List available subagent templates and capabilities",
            Arc::new(JsonObject::default()),
        )
        .with_title("List subagent templates")
        .with_raw_output_schema(list_output_schema),
        Tool::new(
            "list-agents",
            "List all discovered agent definitions from standard locations",
            Arc::new(JsonObject::default()),
        )
        .with_title("List discovered agents")
        .with_raw_output_schema(agents_output_schema),
        Tool::new(
            "run-subagent",
            "Run a subagent with optional backend/template selection",
            run_schema.clone(),
        )
        .with_title("Run a subagent")
        .with_raw_output_schema(run_output_schema.clone()),
        Tool::new(
            "get-run-status",
            "Fetch status for a run",
            run_id_schema.clone(),
        )
        .with_title("Get subagent run status")
        .with_raw_output_schema(run_output_schema.clone()),
        Tool::new(
            "stop-run",
            "Attempt to cancel a running subagent",
            run_id_schema.clone(),
        )
        .with_title("Stop a running subagent")
        .with_raw_output_schema(run_output_schema.clone()),
        Tool::new(
            "get-run-history",
            "Return recent subagent runs",
            history_schema,
        )
        .with_title("Recent runs")
        .with_raw_output_schema(run_output_schema.clone()),
        Tool::new(
            "get-run-events",
            "Poll for events from a run. Use since_index for incremental fetching.",
            events_schema,
        )
        .with_title("Get run events")
        .with_raw_output_schema(events_output_schema),
    ];

    // Codex-only extended tools
    tools.push(
        Tool::new(
            "run-subagent-async",
            "Start background run (Codex-capable backends).",
            run_schema,
        )
        .with_title("Run subagent asynchronously")
        .with_raw_output_schema(run_output_schema.clone()),
    );
    tools.push(
        Tool::new(
            "get-async-status",
            "Fetch status for async runs",
            run_id_schema,
        )
        .with_title("Status for async run")
        .with_raw_output_schema(run_output_schema),
    );
    tools.push(
        Tool::new(
            "download-transcript-secure",
            "Fetch encrypted reasoning transcript (Codex only)",
            Arc::new(JsonObject::default()),
        )
        .with_title("Download secure transcript"),
    );

    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================
    // Schema Generation Tests (BDD style)
    // ==========================================

    mod schema_generation {
        use super::*;

        #[test]
        fn given_run_schema_when_generated_then_has_required_prompt() {
            let schema = run_schema();

            // Schema should require "prompt" field
            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let required = schema_json
                .get("required")
                .expect("required field")
                .as_array()
                .expect("required is array");
            assert!(required.iter().any(|v| v.as_str() == Some("prompt")));
        }

        #[test]
        fn given_run_schema_when_generated_then_has_expected_properties() {
            let schema = run_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let props = schema_json
                .get("properties")
                .expect("properties field")
                .as_object()
                .expect("properties is object");

            // Verify key properties exist
            assert!(props.contains_key("prompt"));
            assert!(props.contains_key("agent_id"));
            assert!(props.contains_key("backend"));
            assert!(props.contains_key("execution_mode"));
            assert!(props.contains_key("timeout_ms"));
        }

        #[test]
        fn given_run_id_schema_when_generated_then_requires_run_id() {
            let schema = run_id_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let required = schema_json
                .get("required")
                .expect("required field")
                .as_array()
                .expect("required is array");
            assert!(required.iter().any(|v| v.as_str() == Some("run_id")));
        }

        #[test]
        fn given_history_schema_when_generated_then_has_limit_property() {
            let schema = history_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let props = schema_json
                .get("properties")
                .expect("properties field")
                .as_object()
                .expect("properties is object");
            assert!(props.contains_key("limit"));

            // Verify limit constraints
            let limit = props.get("limit").expect("limit property");
            assert_eq!(limit.get("minimum"), Some(&serde_json::json!(1)));
            assert_eq!(limit.get("maximum"), Some(&serde_json::json!(50)));
        }

        #[test]
        fn given_events_schema_when_generated_then_has_required_run_id() {
            let schema = events_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let required = schema_json
                .get("required")
                .expect("required field")
                .as_array()
                .expect("required is array");
            assert!(required.iter().any(|v| v.as_str() == Some("run_id")));

            let props = schema_json
                .get("properties")
                .expect("properties field")
                .as_object()
                .expect("properties is object");
            assert!(props.contains_key("since_index"));
        }

        #[test]
        fn given_events_output_schema_when_generated_then_has_events_array() {
            let schema = events_output_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let props = schema_json
                .get("properties")
                .expect("properties field")
                .as_object()
                .expect("properties is object");
            assert!(props.contains_key("events"));
            assert!(props.contains_key("total_count"));
            assert!(props.contains_key("has_more"));
        }

        #[test]
        fn given_run_output_schema_when_generated_then_requires_run_id() {
            let schema = run_output_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let required = schema_json
                .get("required")
                .expect("required field")
                .as_array()
                .expect("required is array");
            assert!(required.iter().any(|v| v.as_str() == Some("run_id")));
        }

        #[test]
        fn given_list_output_schema_when_generated_then_has_templates_array() {
            let schema = list_output_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let props = schema_json
                .get("properties")
                .expect("properties field")
                .as_object()
                .expect("properties is object");
            assert!(props.contains_key("templates"));
        }

        #[test]
        fn given_agents_output_schema_when_generated_then_has_agents_array_with_structure() {
            let schema = agents_output_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let props = schema_json
                .get("properties")
                .expect("properties field")
                .as_object()
                .expect("properties is object");
            assert!(props.contains_key("agents"));

            // Verify agents array item structure
            let agents = props.get("agents").expect("agents property");
            let items = agents.get("items").expect("items field");
            let item_props = items
                .get("properties")
                .expect("item properties")
                .as_object()
                .expect("item properties is object");
            assert!(item_props.contains_key("name"));
            assert!(item_props.contains_key("description"));
            assert!(item_props.contains_key("tools"));
            assert!(item_props.contains_key("requires_cli"));
        }

        #[test]
        fn given_run_schema_when_generated_then_properties_have_correct_types() {
            let schema = run_schema();
            let schema_json = serde_json::to_value(&*schema).expect("schema should serialize");
            let props = schema_json
                .get("properties")
                .expect("schema should have properties")
                .as_object()
                .expect("properties should be object");

            // Verify string types
            assert_eq!(
                props.get("prompt").and_then(|p| p.get("type")),
                Some(&serde_json::json!("string")),
                "prompt should be string type"
            );
            assert_eq!(
                props.get("agent_id").and_then(|p| p.get("type")),
                Some(&serde_json::json!("string")),
                "agent_id should be string type"
            );

            // Verify boolean types
            assert_eq!(
                props.get("tracing").and_then(|p| p.get("type")),
                Some(&serde_json::json!("boolean")),
                "tracing should be boolean type"
            );
            assert_eq!(
                props.get("stream").and_then(|p| p.get("type")),
                Some(&serde_json::json!("boolean")),
                "stream should be boolean type"
            );

            // Verify integer types
            assert_eq!(
                props.get("timeout_ms").and_then(|p| p.get("type")),
                Some(&serde_json::json!("integer")),
                "timeout_ms should be integer type"
            );
        }

        #[test]
        fn given_history_schema_when_generated_then_limit_is_integer_type() {
            let schema = history_schema();
            let schema_json = serde_json::to_value(&*schema).expect("schema should serialize");
            let props = schema_json
                .get("properties")
                .expect("schema should have properties")
                .as_object()
                .expect("properties should be object");

            assert_eq!(
                props.get("limit").and_then(|p| p.get("type")),
                Some(&serde_json::json!("integer")),
                "limit should be integer type"
            );
        }

        #[test]
        fn given_events_output_schema_when_generated_then_has_correct_types() {
            let schema = events_output_schema();
            let schema_json = serde_json::to_value(&*schema).expect("schema should serialize");
            let props = schema_json
                .get("properties")
                .expect("schema should have properties")
                .as_object()
                .expect("properties should be object");

            assert_eq!(
                props.get("total_count").and_then(|p| p.get("type")),
                Some(&serde_json::json!("integer")),
                "total_count should be integer type"
            );
            assert_eq!(
                props.get("has_more").and_then(|p| p.get("type")),
                Some(&serde_json::json!("boolean")),
                "has_more should be boolean type"
            );
            assert_eq!(
                props.get("events").and_then(|p| p.get("type")),
                Some(&serde_json::json!("array")),
                "events should be array type"
            );
        }
    }

    // ==========================================
    // Tool Generation Tests
    // ==========================================

    mod tool_generation {
        use super::*;

        #[test]
        fn given_all_tools_when_generated_then_contains_expected_count() {
            let tools = all_tools();

            // Should have 10 tools total
            assert_eq!(tools.len(), 10);
        }

        #[test]
        fn given_all_tools_when_generated_then_all_have_names() {
            let tools = all_tools();

            for tool in &tools {
                assert!(!tool.name.is_empty(), "Tool should have a name");
            }
        }

        #[test]
        fn given_all_tools_when_generated_then_all_have_descriptions() {
            let tools = all_tools();

            for tool in &tools {
                assert!(
                    tool.description.is_some(),
                    "Tool {} should have description",
                    tool.name
                );
            }
        }

        #[test]
        fn given_all_tools_when_generated_then_contains_core_tools() {
            let tools = all_tools();
            let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();

            assert!(names.contains(&"list-subagents"));
            assert!(names.contains(&"list-agents"));
            assert!(names.contains(&"run-subagent"));
            assert!(names.contains(&"get-run-status"));
            assert!(names.contains(&"stop-run"));
            assert!(names.contains(&"get-run-history"));
            assert!(names.contains(&"get-run-events"));
        }

        #[test]
        fn given_all_tools_when_generated_then_contains_codex_extended_tools() {
            let tools = all_tools();
            let names: Vec<_> = tools.iter().map(|t| t.name.as_ref()).collect();

            assert!(names.contains(&"run-subagent-async"));
            assert!(names.contains(&"get-async-status"));
            assert!(names.contains(&"download-transcript-secure"));
        }

        #[test]
        fn given_list_subagents_tool_when_generated_then_has_empty_input_schema() {
            let tools = all_tools();
            let list_tool = tools
                .iter()
                .find(|t| t.name.as_ref() == "list-subagents")
                .expect("list-subagents tool should exist");

            // list-subagents takes no parameters
            let schema_json =
                serde_json::to_value(&*list_tool.input_schema).expect("schema serialization");
            let schema_obj = schema_json.as_object().expect("schema is object");
            assert!(schema_obj.is_empty() || !schema_obj.contains_key("required"));
        }

        #[test]
        fn given_run_subagent_tool_when_generated_then_has_input_and_output_schemas() {
            let tools = all_tools();
            let run_tool = tools
                .iter()
                .find(|t| t.name.as_ref() == "run-subagent")
                .expect("run-subagent tool should exist");

            // Should have both input and output schemas
            assert!(!run_tool.input_schema.is_empty());
            assert!(run_tool.output_schema.is_some());
        }

        #[test]
        fn given_download_transcript_tool_when_generated_then_has_no_output_schema() {
            let tools = all_tools();
            let transcript_tool = tools
                .iter()
                .find(|t| t.name.as_ref() == "download-transcript-secure")
                .expect("download-transcript-secure tool should exist");

            // download-transcript-secure has no output schema
            assert!(transcript_tool.output_schema.is_none());
        }
    }

    // ==========================================
    // Schema Validation Tests
    // ==========================================

    mod schema_validation {
        use super::*;

        #[test]
        fn given_timeout_ms_in_run_schema_when_validated_then_has_bounds() {
            let schema = run_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let props = schema_json.get("properties").expect("properties field");
            let timeout = props.get("timeout_ms").expect("timeout_ms property");

            assert_eq!(timeout.get("minimum"), Some(&serde_json::json!(1)));
            assert_eq!(timeout.get("maximum"), Some(&serde_json::json!(300000)));
        }

        #[test]
        fn given_since_index_in_events_schema_when_validated_then_has_minimum() {
            let schema = events_schema();

            let schema_json = serde_json::to_value(&*schema).expect("schema serialization");
            let props = schema_json.get("properties").expect("properties field");
            let since_index = props.get("since_index").expect("since_index property");

            assert_eq!(since_index.get("minimum"), Some(&serde_json::json!(0)));
        }

        #[test]
        fn given_all_schemas_when_serialized_then_valid_json() {
            // Verify all schemas can be serialized to valid JSON
            let schemas: Vec<Arc<JsonObject>> = vec![
                run_schema(),
                run_id_schema(),
                history_schema(),
                events_schema(),
                events_output_schema(),
                run_output_schema(),
                list_output_schema(),
                agents_output_schema(),
            ];

            for schema in schemas {
                let json = serde_json::to_string(&*schema);
                assert!(json.is_ok(), "Schema should serialize to valid JSON");
            }
        }
    }
}
