//! MCP server configuration reading and writing for Cursor adapter.
//!
//! Cursor stores MCP config in `.cursor/mcp.json`, similar to Claude's `.mcp.json`.

use super::paths::mcp_config_path;
use crate::common::{McpServer, McpTransport};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::{debug, warn};

/// Cursor's MCP config structure (mirrors Claude's format).
#[derive(Debug, Serialize, Deserialize, Default)]
struct CursorMcpConfig {
    #[serde(rename = "mcpServers", default)]
    mcp_servers: HashMap<String, McpServerEntry>,
}

/// A single MCP server entry in Cursor's config.
#[derive(Debug, Serialize, Deserialize)]
struct McpServerEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    headers: Option<HashMap<String, String>>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    enabled: bool,
}

fn default_true() -> bool {
    true
}

fn is_true(v: &bool) -> bool {
    *v
}

/// Reads MCP server configs from `.cursor/mcp.json`.
pub fn read_mcp_servers(root: &Path) -> Result<HashMap<String, McpServer>> {
    let path = mcp_config_path(root);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path)?;
    let config: CursorMcpConfig = serde_json::from_str(&content)?;

    let mut servers = HashMap::new();
    for (name, entry) in config.mcp_servers {
        // Skip entries with neither command nor URL — they produce broken configs
        if entry.command.is_none() && entry.url.is_none() {
            warn!(
                name = %name,
                "Skipping MCP entry with neither command nor url"
            );
            continue;
        }

        let transport = if entry.url.is_some() {
            McpTransport::Http
        } else {
            McpTransport::Stdio
        };

        servers.insert(
            name.clone(),
            McpServer {
                name,
                transport,
                command: entry.command.unwrap_or_default(),
                args: entry.args,
                env: entry.env,
                url: entry.url,
                headers: entry.headers.clone(),
                enabled: entry.enabled,
            },
        );
    }

    Ok(servers)
}

/// Writes MCP server configs to `.cursor/mcp.json`.
pub fn write_mcp_servers(root: &Path, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
    let mut report = WriteReport::default();

    if servers.is_empty() {
        return Ok(report);
    }

    let mut config = CursorMcpConfig::default();

    for (name, server) in servers {
        let entry = McpServerEntry {
            command: if server.command.is_empty() {
                None
            } else {
                Some(server.command.clone())
            },
            args: server.args.clone(),
            env: server.env.clone(),
            url: server.url.clone(),
            headers: server.headers.clone(),
            enabled: server.enabled,
        };

        debug!(name = %name, "Writing Cursor MCP server");
        config.mcp_servers.insert(name.clone(), entry);
    }

    let json = serde_json::to_string_pretty(&config)?;

    // Skip write if existing config matches
    let path = mcp_config_path(root);
    if path.exists() {
        if let Ok(existing) = fs::read_to_string(&path) {
            if existing == json {
                report.skipped.push(SkipReason::Unchanged {
                    item: "mcp.json".to_string(),
                });
                return Ok(report);
            }
        }
    }

    report.written = servers.len();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, &json)?;

    Ok(report)
}
