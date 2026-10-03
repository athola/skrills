//! Shared reading, encoding and merging of JSON MCP server maps.
//!
//! Claude (`settings.json`), Codex (`config.json`), Copilot
//! (`mcp-config.json`) and Cursor (`mcp.json`) all keep MCP servers as a JSON
//! object keyed by server name under `mcpServers`. Each writer used to build a
//! fresh object from the source and assign it over the target's, so every
//! server that only the target knew about was deleted on each sync, and a
//! source with no servers emptied the target. Writers now merge through
//! [`merge_servers`].

use crate::common::{McpServer, McpTransport};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use anyhow::anyhow;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::Path;

/// Per-server keys a writer owns. A key outside this list (a newer schema
/// field, a comment-like `_note`) is left as the user wrote it.
const MANAGED_KEYS: &[&str] = &[
    "type",
    "command",
    "args",
    "env",
    "url",
    "headers",
    "disabled",
    "enabled",
    "allowedTools",
    "disabledTools",
];

/// How a target spells the fields that differ between tools.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dialect {
    /// Claude: `type`/`url`/`headers` for HTTP, `disabled: true`.
    Claude,
    /// Codex and Copilot: stdio only, `disabled: true`.
    StdioOnly,
    /// Cursor: `url`/`headers` without `type`, `enabled: false`.
    Cursor,
}

/// Reads a JSON config file whose root must be an object, or `{}` when the
/// file does not exist.
///
/// A root that parses but is not an object (`[]`, a bare string) used to reach
/// `value["key"] = ...`, which panics inside the MCP server process.
pub(crate) fn load_object(path: &Path) -> Result<Value> {
    let value = match std::fs::read_to_string(path) {
        Ok(content) => serde_json::from_str::<Value>(&content)
            .map_err(|e| anyhow!("Failed to parse {} as JSON: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(e) => return Err(anyhow!("Failed to read {}: {e}", path.display())),
    };
    if !value.is_object() {
        return Err(anyhow!(
            "Expected a JSON object at the root of {}, found {}; refusing to overwrite it",
            path.display(),
            json_kind(&value)
        ));
    }
    Ok(value)
}

/// Serializes `config` and writes it through
/// [`crate::adapters::utils::write_config`]: atomic, backed up, and a no-op
/// when nothing changed. Returns whether the file was written.
///
/// MCP configs carry `env` and `headers` secrets, so a newly created file is
/// private (`0o600`).
pub(crate) fn write_json_config(path: &Path, config: &Value) -> Result<bool> {
    let json = serde_json::to_string_pretty(config)?;
    crate::adapters::utils::write_config(path, json.as_bytes(), true)
        .map_err(|e| anyhow!("Failed to write {}: {e}", path.display()))
}

/// Sets the top-level `model` key of a JSON config, leaving every other key
/// as it was.
///
/// With no model to set the file is not touched at all: it used to be
/// re-serialized (and created when missing) on every sync.
pub(crate) fn write_model(path: &Path, model: Option<&str>) -> Result<WriteReport> {
    let mut report = WriteReport::default();
    let Some(model) = model else {
        return Ok(report);
    };
    let mut config = load_object(path)?;
    if config.get("model").and_then(Value::as_str) == Some(model) {
        report.skipped.push(SkipReason::Unchanged {
            item: "model".to_string(),
        });
        return Ok(report);
    }
    config
        .as_object_mut()
        .expect("load_object returns an object")
        .insert("model".into(), Value::from(model));
    write_json_config(path, &config)?;
    report.written += 1;
    Ok(report)
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Encodes one server in `dialect`, or `None` when the target cannot express
/// it (an HTTP server for a stdio-only tool).
pub(crate) fn encode_server(server: &McpServer, dialect: Dialect) -> Option<Map<String, Value>> {
    let mut entry = Map::new();
    match server.transport {
        McpTransport::Http => {
            if dialect == Dialect::StdioOnly {
                return None;
            }
            if dialect == Dialect::Claude {
                entry.insert("type".into(), Value::from("http"));
            }
            if let Some(url) = &server.url {
                entry.insert("url".into(), Value::from(url.as_str()));
            }
            if let Some(headers) = &server.headers {
                entry.insert("headers".into(), string_map(headers));
            }
        }
        McpTransport::Stdio => {
            entry.insert("command".into(), Value::from(server.command.as_str()));
            if !server.args.is_empty() {
                entry.insert("args".into(), Value::from(server.args.clone()));
            }
            if !server.env.is_empty() {
                entry.insert("env".into(), string_map(&server.env));
            }
        }
    }
    if !server.enabled {
        match dialect {
            Dialect::Cursor => entry.insert("enabled".into(), Value::Bool(false)),
            _ => entry.insert("disabled".into(), Value::Bool(true)),
        };
    }
    if !server.allowed_tools.is_empty() {
        entry.insert(
            "allowedTools".into(),
            Value::from(server.allowed_tools.clone()),
        );
    }
    if !server.disabled_tools.is_empty() {
        entry.insert(
            "disabledTools".into(),
            Value::from(server.disabled_tools.clone()),
        );
    }
    Some(entry)
}

fn string_map(map: &HashMap<String, String>) -> Value {
    let mut sorted: Vec<_> = map.iter().collect();
    sorted.sort();
    Value::Object(
        sorted
            .into_iter()
            .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
            .collect(),
    )
}

/// Merges `servers` into `config.mcpServers` by name.
///
/// Source servers are inserted or updated; servers only the target has are
/// kept, and so are unknown keys inside an updated entry. A server the target
/// cannot express is recorded as skipped. Returns the report; `config` is
/// changed in place and the caller decides whether to write it.
pub(crate) fn merge_servers(
    config: &mut Value,
    servers: &HashMap<String, McpServer>,
    dialect: Dialect,
    target_name: &str,
) -> Result<WriteReport> {
    let mut report = WriteReport::default();
    let root = config
        .as_object_mut()
        .ok_or_else(|| anyhow!("MCP config root is not a JSON object"))?;
    let existing = root
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()));
    if !existing.is_object() {
        return Err(anyhow!(
            "`mcpServers` in the {target_name} config is {}, not an object; refusing to replace it",
            json_kind(existing)
        ));
    }
    let map = existing.as_object_mut().expect("checked above");

    let mut names: Vec<_> = servers.keys().collect();
    names.sort();
    for name in names {
        let server = &servers[name];
        let Some(encoded) = encode_server(server, dialect) else {
            report.skipped.push(SkipReason::AgentSpecificFeature {
                item: name.clone(),
                feature: "HTTP MCP transport".to_string(),
                suggestion: format!(
                    "{target_name} only runs stdio MCP servers; add this server there by hand"
                ),
            });
            continue;
        };
        let mut merged = match map.get(name) {
            Some(Value::Object(current)) => current.clone(),
            _ => Map::new(),
        };
        for key in MANAGED_KEYS {
            merged.remove(*key);
        }
        merged.extend(encoded);
        let merged = Value::Object(merged);
        if map.get(name) == Some(&merged) {
            report
                .skipped
                .push(SkipReason::Unchanged { item: name.clone() });
        } else {
            map.insert(name.clone(), merged);
            report.written += 1;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(name: &str, command: &str) -> McpServer {
        McpServer {
            name: name.to_string(),
            transport: McpTransport::Stdio,
            command: command.to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            url: None,
            headers: None,
            enabled: true,
            allowed_tools: Vec::new(),
            disabled_tools: Vec::new(),
        }
    }

    #[test]
    fn merge_keeps_target_only_servers_and_unknown_fields() {
        let mut config = serde_json::json!({
            "theme": "dark",
            "mcpServers": {
                "a": {"command": "a-bin"},
                "c": {"command": "old", "timeout": 30}
            }
        });
        let servers = HashMap::from([("c".to_string(), stdio("c", "new"))]);

        let report = merge_servers(&mut config, &servers, Dialect::Claude, "claude").unwrap();

        assert_eq!(report.written, 1);
        assert_eq!(config["theme"], "dark");
        assert_eq!(config["mcpServers"]["a"]["command"], "a-bin");
        assert_eq!(config["mcpServers"]["c"]["command"], "new");
        assert_eq!(config["mcpServers"]["c"]["timeout"], 30);
    }

    #[test]
    fn merge_with_no_source_servers_changes_nothing() {
        let mut config = serde_json::json!({"mcpServers": {"a": {"command": "a"}}});
        let before = config.clone();
        merge_servers(&mut config, &HashMap::new(), Dialect::StdioOnly, "codex").unwrap();
        assert_eq!(config, before);
    }

    #[test]
    fn stdio_only_target_skips_http_servers_instead_of_writing_an_empty_command() {
        let mut http = stdio("remote", "");
        http.transport = McpTransport::Http;
        http.url = Some("https://x/mcp".to_string());
        let mut config = serde_json::json!({});
        let servers = HashMap::from([("remote".to_string(), http)]);

        let report = merge_servers(&mut config, &servers, Dialect::StdioOnly, "codex").unwrap();

        assert_eq!(report.written, 0);
        assert!(config["mcpServers"].get("remote").is_none());
        assert!(matches!(
            report.skipped[0],
            SkipReason::AgentSpecificFeature { .. }
        ));
    }

    #[test]
    fn unchanged_servers_are_reported_unchanged() {
        let mut config = serde_json::json!({"mcpServers": {"a": {"command": "a"}}});
        let servers = HashMap::from([("a".to_string(), stdio("a", "a"))]);
        let report = merge_servers(&mut config, &servers, Dialect::Claude, "claude").unwrap();
        assert_eq!(report.written, 0);
        assert!(matches!(report.skipped[0], SkipReason::Unchanged { .. }));
    }

    #[test]
    fn load_object_rejects_a_non_object_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, "[]").unwrap();
        let err = load_object(&path).unwrap_err().to_string();
        assert!(err.contains("Expected a JSON object"), "{err}");
    }

    #[test]
    fn load_object_defaults_a_missing_file_to_an_empty_object() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_object(&dir.path().join("none.json")).unwrap(),
            serde_json::json!({})
        );
    }
}
