//! MCP Servers API endpoints.
//!
//! REST API for discovering MCP server configurations across all CLI adapters.
//!
//! ## Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/api/mcp-servers` | List MCP servers from all adapters |

use axum::{routing::get, Json, Router};
use serde::Serialize;
use skrills_sync::adapters::traits::AgentAdapter;
use skrills_sync::common::McpTransport;
use skrills_sync::{ClaudeAdapter, CodexAdapter, CopilotAdapter, CursorAdapter};
use std::collections::HashMap;

/// MCP server info for API response.
#[derive(Debug, Serialize)]
pub struct McpServerResponse {
    pub name: String,
    pub source: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub enabled: bool,
    pub allowed_tools: Vec<String>,
    pub disabled_tools: Vec<String>,
}

/// Aggregated response for all MCP servers.
#[derive(Debug, Serialize)]
pub struct McpServersListResponse {
    pub servers: Vec<McpServerResponse>,
    pub total: usize,
}

/// Read MCP servers from a single adapter, tagging each with the source name.
fn collect_from_adapter(
    adapter: &dyn AgentAdapter,
    source: &str,
    out: &mut Vec<McpServerResponse>,
) {
    if let Ok(servers) = adapter.read_mcp_servers() {
        for (name, server) in servers {
            out.push(McpServerResponse {
                name,
                source: source.to_string(),
                transport: match server.transport {
                    McpTransport::Stdio => "stdio".to_string(),
                    McpTransport::Http => "http".to_string(),
                },
                command: server.command,
                args: server.args,
                env: server.env,
                url: server.url,
                enabled: server.enabled,
                allowed_tools: server.allowed_tools,
                disabled_tools: server.disabled_tools,
            });
        }
    }
}

/// List MCP servers from all adapters.
async fn list_mcp_servers() -> Json<McpServersListResponse> {
    let mut servers = Vec::new();

    if let Ok(adapter) = ClaudeAdapter::new() {
        collect_from_adapter(&adapter, "claude", &mut servers);
    }
    if let Ok(adapter) = CodexAdapter::new() {
        collect_from_adapter(&adapter, "codex", &mut servers);
    }
    if let Ok(adapter) = CopilotAdapter::new() {
        collect_from_adapter(&adapter, "copilot", &mut servers);
    }
    if let Ok(adapter) = CursorAdapter::new() {
        collect_from_adapter(&adapter, "cursor", &mut servers);
    }

    servers.sort_by(|a, b| a.source.cmp(&b.source).then(a.name.cmp(&b.name)));
    let total = servers.len();

    Json(McpServersListResponse { servers, total })
}

/// Create MCP servers API routes.
pub fn mcp_servers_routes() -> Router {
    Router::new().route("/api/mcp-servers", get(list_mcp_servers))
}
