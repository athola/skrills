//! settings.json reading/writing for the Claude adapter.
//!
//! Combines MCP server roundtripping (transport, auth, tool gates)
//! and the lightweight Preferences struct (currently only `model`).
//! Both subjects share `~/.claude/settings.json` as their backing
//! store, which is why they live in one module.

use crate::adapters::json_config::{self, Dialect};
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
                    // Read as stdio, an `sse` server would later be written back
                    // with an empty command and no URL. Leaving it out keeps the
                    // target's own copy, since writers merge by name.
                    tracing::warn!(
                        unknown_type = other,
                        name = %name,
                        "Skipping MCP server with a transport sync cannot carry"
                    );
                    continue;
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
    let mut settings = json_config::load_object(&path)?;
    let report = json_config::merge_servers(&mut settings, servers, Dialect::Claude, "claude")?;
    if report.written > 0 {
        json_config::write_json_config(&path, &settings)?;
    }
    Ok(report)
}

pub(super) fn write_preferences_impl(
    adapter: &ClaudeAdapter,
    prefs: &Preferences,
) -> Result<WriteReport> {
    json_config::write_model(&adapter.settings_path(), prefs.model.as_deref())
}
