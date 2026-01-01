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
