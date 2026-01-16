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
        Tool {
            name: "list-subagents".into(),
            title: Some("List subagent templates".into()),
            description: Some("List available subagent templates and capabilities".into()),
            input_schema: Arc::new(JsonObject::default()),
            output_schema: Some(list_output_schema),
            annotations: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "list-agents".into(),
            title: Some("List discovered agents".into()),
            description: Some(
                "List all discovered agent definitions from standard locations".into(),
            ),
            input_schema: Arc::new(JsonObject::default()),
            output_schema: Some(agents_output_schema),
            annotations: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "run-subagent".into(),
            title: Some("Run a subagent".into()),
            description: Some("Run a subagent with optional backend/template selection".into()),
            input_schema: run_schema.clone(),
            output_schema: Some(run_output_schema.clone()),
            annotations: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "get-run-status".into(),
            title: Some("Get subagent run status".into()),
            description: Some("Fetch status for a run".into()),
            input_schema: run_id_schema.clone(),
            output_schema: Some(run_output_schema.clone()),
            annotations: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "stop-run".into(),
            title: Some("Stop a running subagent".into()),
            description: Some("Attempt to cancel a running subagent".into()),
            input_schema: run_id_schema.clone(),
            output_schema: Some(run_output_schema.clone()),
            annotations: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "get-run-history".into(),
            title: Some("Recent runs".into()),
            description: Some("Return recent subagent runs".into()),
            input_schema: history_schema,
            output_schema: Some(run_output_schema.clone()),
            annotations: None,
            icons: None,
            meta: None,
        },
        Tool {
            name: "get-run-events".into(),
            title: Some("Get run events".into()),
            description: Some(
                "Poll for events from a run. Use since_index for incremental fetching.".into(),
            ),
            input_schema: events_schema,
            output_schema: Some(events_output_schema),
            annotations: None,
            icons: None,
            meta: None,
        },
    ];

    // Codex-only extended tools
    tools.push(Tool {
        name: "run-subagent-async".into(),
        title: Some("Run subagent asynchronously".into()),
        description: Some("Start background run (Codex-capable backends).".into()),
        input_schema: run_schema,
        output_schema: Some(run_output_schema.clone()),
        annotations: None,
        icons: None,
        meta: None,
    });
    tools.push(Tool {
        name: "get-async-status".into(),
        title: Some("Status for async run".into()),
        description: Some("Fetch status for async runs".into()),
        input_schema: run_id_schema,
        output_schema: Some(run_output_schema),
        annotations: None,
        icons: None,
        meta: None,
    });
    tools.push(Tool {
        name: "download-transcript-secure".into(),
        title: Some("Download secure transcript".into()),
        description: Some("Fetch encrypted reasoning transcript (Codex only)".into()),
        input_schema: Arc::new(JsonObject::default()),
        output_schema: None,
        annotations: None,
        icons: None,
        meta: None,
    });

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
            let schema_json = serde_json::to_value(&*schema).unwrap();
            let required = schema_json.get("required").unwrap().as_array().unwrap();
            assert!(required.iter().any(|v| v.as_str() == Some("prompt")));
        }

        #[test]
        fn given_run_schema_when_generated_then_has_expected_properties() {
            let schema = run_schema();

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let props = schema_json.get("properties").unwrap().as_object().unwrap();

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

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let required = schema_json.get("required").unwrap().as_array().unwrap();
            assert!(required.iter().any(|v| v.as_str() == Some("run_id")));
        }

        #[test]
        fn given_history_schema_when_generated_then_has_limit_property() {
            let schema = history_schema();

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let props = schema_json.get("properties").unwrap().as_object().unwrap();
            assert!(props.contains_key("limit"));

            // Verify limit constraints
            let limit = props.get("limit").unwrap();
            assert_eq!(limit.get("minimum"), Some(&serde_json::json!(1)));
            assert_eq!(limit.get("maximum"), Some(&serde_json::json!(50)));
        }

        #[test]
        fn given_events_schema_when_generated_then_has_required_run_id() {
            let schema = events_schema();

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let required = schema_json.get("required").unwrap().as_array().unwrap();
            assert!(required.iter().any(|v| v.as_str() == Some("run_id")));

            let props = schema_json.get("properties").unwrap().as_object().unwrap();
            assert!(props.contains_key("since_index"));
        }

        #[test]
        fn given_events_output_schema_when_generated_then_has_events_array() {
            let schema = events_output_schema();

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let props = schema_json.get("properties").unwrap().as_object().unwrap();
            assert!(props.contains_key("events"));
            assert!(props.contains_key("total_count"));
            assert!(props.contains_key("has_more"));
        }

        #[test]
        fn given_run_output_schema_when_generated_then_requires_run_id() {
            let schema = run_output_schema();

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let required = schema_json.get("required").unwrap().as_array().unwrap();
            assert!(required.iter().any(|v| v.as_str() == Some("run_id")));
        }

        #[test]
        fn given_list_output_schema_when_generated_then_has_templates_array() {
            let schema = list_output_schema();

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let props = schema_json.get("properties").unwrap().as_object().unwrap();
            assert!(props.contains_key("templates"));
        }

        #[test]
        fn given_agents_output_schema_when_generated_then_has_agents_array_with_structure() {
            let schema = agents_output_schema();

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let props = schema_json.get("properties").unwrap().as_object().unwrap();
            assert!(props.contains_key("agents"));

            // Verify agents array item structure
            let agents = props.get("agents").unwrap();
            let items = agents.get("items").unwrap();
            let item_props = items.get("properties").unwrap().as_object().unwrap();
            assert!(item_props.contains_key("name"));
            assert!(item_props.contains_key("description"));
            assert!(item_props.contains_key("tools"));
            assert!(item_props.contains_key("requires_cli"));
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
                .unwrap();

            // list-subagents takes no parameters
            let schema_json = serde_json::to_value(&*list_tool.input_schema).unwrap();
            assert!(
                schema_json.as_object().unwrap().is_empty()
                    || !schema_json.as_object().unwrap().contains_key("required")
            );
        }

        #[test]
        fn given_run_subagent_tool_when_generated_then_has_input_and_output_schemas() {
            let tools = all_tools();
            let run_tool = tools
                .iter()
                .find(|t| t.name.as_ref() == "run-subagent")
                .unwrap();

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
                .unwrap();

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

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let props = schema_json.get("properties").unwrap();
            let timeout = props.get("timeout_ms").unwrap();

            assert_eq!(timeout.get("minimum"), Some(&serde_json::json!(1)));
            assert_eq!(timeout.get("maximum"), Some(&serde_json::json!(300000)));
        }

        #[test]
        fn given_since_index_in_events_schema_when_validated_then_has_minimum() {
            let schema = events_schema();

            let schema_json = serde_json::to_value(&*schema).unwrap();
            let props = schema_json.get("properties").unwrap();
            let since_index = props.get("since_index").unwrap();

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
