//! MCP server reading and writing for Copilot adapter.

use super::paths::mcp_config_path;
use crate::adapters::json_config::{self, Dialect};
use crate::common::{McpServer, McpTransport};
use crate::report::WriteReport;
use crate::Result;
use anyhow::Context;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::warn;

/// Reads MCP servers from the mcp-config.json file.
pub fn read_mcp_servers(root: &Path) -> Result<HashMap<String, McpServer>> {
    let path = mcp_config_path(root);
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read MCP config: {}", path.display()))?;
    let config: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse MCP config as JSON: {}", path.display()))?;

    let mut servers = HashMap::new();
    if let Some(mcp) = config.get("mcpServers").and_then(|v| v.as_object()) {
        for (name, server_config) in mcp {
            // Skip MCP servers with missing or empty command and log a warning
            let command = server_config
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            if command.is_empty() {
                // Logged only: a library printing to stderr corrupted the TUI.
                warn!(
                    server = %name,
                    path = %path.display(),
                    "Skipping MCP server with missing or empty 'command' field"
                );
                continue;
            }

            // Parse args with warnings for wrong types
            let args = match server_config.get("args") {
                Some(v) if v.is_array() => {
                    let mut result = Vec::new();
                    for (i, item) in v.as_array().unwrap().iter().enumerate() {
                        if let Some(s) = item.as_str() {
                            result.push(s.to_string());
                        } else {
                            warn!(
                                server = %name,
                                index = i,
                                value_type = ?item,
                                "Skipping non-string value in MCP server args"
                            );
                        }
                    }
                    result
                }
                Some(v) => {
                    // args exists but is wrong type (e.g., string instead of array)
                    warn!(
                        server = %name,
                        expected = "array",
                        actual = ?v,
                        "MCP server 'args' has wrong type, expected array"
                    );
                    Vec::new()
                }
                None => Vec::new(),
            };

            // Parse env with warnings for wrong types
            let env = match server_config.get("env") {
                Some(v) if v.is_object() => {
                    let mut result = HashMap::new();
                    for (k, val) in v.as_object().unwrap() {
                        if let Some(s) = val.as_str() {
                            result.insert(k.clone(), s.to_string());
                        } else {
                            warn!(
                                server = %name,
                                key = %k,
                                value_type = ?val,
                                "Skipping non-string value in MCP server env"
                            );
                        }
                    }
                    result
                }
                Some(v) => {
                    // env exists but is wrong type (e.g., array instead of object)
                    warn!(
                        server = %name,
                        expected = "object",
                        actual = ?v,
                        "MCP server 'env' has wrong type, expected object"
                    );
                    HashMap::new()
                }
                None => HashMap::new(),
            };

            let server = McpServer {
                name: name.clone(),
                transport: McpTransport::Stdio, // Copilot only supports stdio
                command: command.to_string(),
                args,
                env,
                url: None,     // Copilot doesn't support HTTP
                headers: None, // Copilot doesn't support HTTP
                enabled: server_config
                    .get("disabled")
                    .and_then(|v| v.as_bool())
                    .map(|d| !d)
                    .unwrap_or(true),
                allowed_tools: server_config
                    .get("allowedTools")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                disabled_tools: server_config
                    .get("disabledTools")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            };
            servers.insert(name.clone(), server);
        }
    }

    Ok(servers)
}

/// Writes MCP servers to the mcp-config.json file, merged by name into what
/// is already there.
pub fn write_mcp_servers(root: &Path, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
    let path = mcp_config_path(root);
    let mut config = json_config::load_object(&path)?;
    let report = json_config::merge_servers(&mut config, servers, Dialect::StdioOnly, "copilot")?;
    if report.written > 0 {
        json_config::write_json_config(&path, &config)?;
    }
    Ok(report)
}
