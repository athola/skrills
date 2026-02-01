//! MCP server reading and writing for Copilot adapter.

use super::paths::mcp_config_path;
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
                eprintln!(
                    "warning: skipping MCP server '{}' (missing or empty 'command' field in {})",
                    name,
                    path.display()
                );
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
            };
            servers.insert(name.clone(), server);
        }
    }

    Ok(servers)
}

/// Writes MCP servers to the mcp-config.json file.
pub fn write_mcp_servers(root: &Path, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
    let path = mcp_config_path(root);

    // Read existing config to preserve structure
    let mut config: serde_json::Value = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read MCP config: {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse MCP config as JSON: {}", path.display()))?
    } else {
        serde_json::json!({})
    };

    let mut report = WriteReport::default();
    let mut mcp_obj = serde_json::Map::new();

    for (name, server) in servers {
        let mut server_config = serde_json::Map::new();
        server_config.insert("command".into(), serde_json::json!(server.command));
        if !server.args.is_empty() {
            server_config.insert("args".into(), serde_json::json!(server.args));
        }
        if !server.env.is_empty() {
            server_config.insert("env".into(), serde_json::json!(server.env));
        }
        if !server.enabled {
            server_config.insert("disabled".into(), serde_json::json!(true));
        }
        mcp_obj.insert(name.clone(), serde_json::Value::Object(server_config));
        report.written += 1;
    }

    config["mcpServers"] = serde_json::Value::Object(mcp_obj);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create MCP config directory: {}",
                parent.display()
            )
        })?;
    }
    fs::write(&path, serde_json::to_string_pretty(&config)?)
        .with_context(|| format!("Failed to write MCP config: {}", path.display()))?;

    Ok(report)
}
