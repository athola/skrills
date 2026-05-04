//! settings.json reading/writing for the Claude adapter.
//!
//! Combines MCP server roundtripping (transport, auth, tool gates)
//! and the lightweight Preferences struct (currently only `model`).
//! Both subjects share `~/.claude/settings.json` as their backing
//! store, which is why they live in one module.

use crate::common::{McpServer, McpTransport, Preferences};
use crate::report::WriteReport;
use crate::Result;

use std::collections::HashMap;
use std::fs;

use super::ClaudeAdapter;

pub(super) fn read_mcp_servers_impl(adapter: &ClaudeAdapter) -> Result<HashMap<String, McpServer>> {
    let path = adapter.settings_path();
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path)?;
    let settings: serde_json::Value = serde_json::from_str(&content)?;

    let mut servers = HashMap::new();
    if let Some(mcp) = settings.get("mcpServers").and_then(|v| v.as_object()) {
        for (name, config) in mcp {
            // Determine transport type from "type" field (default to stdio)
            let transport = match config.get("type").and_then(|v| v.as_str()) {
                Some("http") => McpTransport::Http,
                Some("stdio") | None => McpTransport::Stdio,
                Some(other) => {
                    tracing::warn!(unknown_type = other, name = %name, "Unknown MCP server type, defaulting to stdio");
                    McpTransport::Stdio
                }
            };

            let server = McpServer {
                name: name.clone(),
                transport,
                command: config
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                args: config
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                env: config
                    .get("env")
                    .and_then(|v| v.as_object())
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default(),
                url: config.get("url").and_then(|v| v.as_str()).map(String::from),
                headers: config
                    .get("headers")
                    .and_then(|v| v.as_object())
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                            .collect()
                    }),
                enabled: config
                    .get("disabled")
                    .and_then(|v| v.as_bool())
                    .map(|d| !d)
                    .unwrap_or(true),
                allowed_tools: config
                    .get("allowedTools")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                disabled_tools: config
                    .get("disabledTools")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            };

            // Warn if HTTP transport is missing URL (required for HTTP servers)
            if server.transport == McpTransport::Http && server.url.is_none() {
                tracing::warn!(
                    name = %name,
                    "HTTP MCP server is missing required 'url' field"
                );
            }

            servers.insert(name.clone(), server);
        }
    }

    Ok(servers)
}

pub(super) fn read_preferences_impl(adapter: &ClaudeAdapter) -> Result<Preferences> {
    let path = adapter.settings_path();
    if !path.exists() {
        return Ok(Preferences::default());
    }

    let content = fs::read_to_string(&path)?;
    let settings: serde_json::Value = serde_json::from_str(&content)?;

    Ok(Preferences {
        model: settings
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from),
        custom: HashMap::new(), // Could extract other fields here
    })
}

pub(super) fn write_mcp_servers_impl(
    adapter: &ClaudeAdapter,
    servers: &HashMap<String, McpServer>,
) -> Result<WriteReport> {
    let path = adapter.settings_path();

    // Read existing settings or create new
    let mut settings: serde_json::Value = if path.exists() {
        let content = fs::read_to_string(&path)?;
        serde_json::from_str(&content)?
    } else {
        serde_json::json!({})
    };

    let mut report = WriteReport::default();
    let mut mcp_obj = serde_json::Map::new();

    for (name, server) in servers {
        let mut server_config = serde_json::Map::new();

        // Write transport type (only for non-stdio to keep config clean)
        if server.transport != McpTransport::Stdio {
            server_config.insert(
                "type".into(),
                serde_json::json!(match server.transport {
                    McpTransport::Stdio => "stdio",
                    McpTransport::Http => "http",
                }),
            );
        }

        match server.transport {
            McpTransport::Http => {
                // HTTP transport: write url and headers
                if let Some(ref url) = server.url {
                    server_config.insert("url".into(), serde_json::json!(url));
                }
                if let Some(ref headers) = server.headers {
                    let headers_obj: serde_json::Map<String, serde_json::Value> = headers
                        .iter()
                        .map(|(k, v)| (k.clone(), serde_json::json!(v)))
                        .collect();
                    server_config.insert("headers".into(), serde_json::Value::Object(headers_obj));
                }
            }
            McpTransport::Stdio => {
                // stdio transport: write command, args, env
                server_config.insert("command".into(), serde_json::json!(server.command));
                if !server.args.is_empty() {
                    server_config.insert("args".into(), serde_json::json!(server.args));
                }
                if !server.env.is_empty() {
                    server_config.insert("env".into(), serde_json::json!(server.env));
                }
            }
        }

        if !server.enabled {
            server_config.insert("disabled".into(), serde_json::json!(true));
        }
        if !server.allowed_tools.is_empty() {
            server_config.insert(
                "allowedTools".into(),
                serde_json::json!(server.allowed_tools),
            );
        }
        if !server.disabled_tools.is_empty() {
            server_config.insert(
                "disabledTools".into(),
                serde_json::json!(server.disabled_tools),
            );
        }
        mcp_obj.insert(name.clone(), serde_json::Value::Object(server_config));
        report.written += 1;
    }

    settings["mcpServers"] = serde_json::Value::Object(mcp_obj);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&settings)?)?;

    Ok(report)
}

pub(super) fn write_preferences_impl(
    adapter: &ClaudeAdapter,
    prefs: &Preferences,
) -> Result<WriteReport> {
    let path = adapter.settings_path();

    let mut settings: serde_json::Value = if path.exists() {
        let content = fs::read_to_string(&path)?;
        serde_json::from_str(&content)?
    } else {
        serde_json::json!({})
    };

    let mut report = WriteReport::default();

    if let Some(model) = &prefs.model {
        settings["model"] = serde_json::json!(model);
        report.written += 1;
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serde_json::to_string_pretty(&settings)?)?;

    Ok(report)
}
