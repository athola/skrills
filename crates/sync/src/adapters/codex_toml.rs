//! Reading and editing Codex's `~/.codex/config.toml`.
//!
//! Codex reads MCP servers from `[mcp_servers.<name>]` tables and the model
//! from a top-level `model` key in this file. Edits go through `toml_edit`, so
//! comments, key order and tables sync does not manage are kept as written.
//!
//! A server may already be spelled as a `[mcp_servers.<name>]` table, with
//! dotted keys, or as an inline table. Each is updated where it is: adding a
//! second definition of the same table makes the whole file invalid, and Codex
//! then refuses to start.

use crate::common::{McpServer, McpTransport};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use anyhow::anyhow;
use std::collections::HashMap;
use std::path::Path;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

/// Parses `path`, or returns an empty document when it does not exist.
///
/// A file that does not parse is an error: it must never be replaced by a
/// document built from scratch.
pub(crate) fn load(path: &Path) -> Result<DocumentMut> {
    match std::fs::read_to_string(path) {
        Ok(content) => content.parse::<DocumentMut>().map_err(|e| {
            anyhow!(
                "Failed to parse {} as TOML; refusing to change it: {e}",
                path.display()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(anyhow!("Failed to read {}: {e}", path.display())),
    }
}

/// Writes `doc` through [`crate::adapters::utils::write_config`]: atomic,
/// with a one-generation `.skrills-bak`, and a no-op when nothing changed.
///
/// Server tables can carry `env` secrets, so a newly created file is private.
pub(crate) fn save(path: &Path, doc: &DocumentMut) -> Result<bool> {
    crate::adapters::utils::write_config(path, doc.to_string().as_bytes(), true)
        .map_err(|e| anyhow!("Failed to write {}: {e}", path.display()))
}

/// The top-level `model`, if it is a string.
pub(crate) fn read_model(doc: &DocumentMut) -> Option<String> {
    doc.get("model")
        .and_then(Item::as_str)
        .map(ToString::to_string)
}

/// Sets the top-level `model`. Returns whether the document changed.
pub(crate) fn set_model(doc: &mut DocumentMut, model: &str) -> bool {
    if read_model(doc).as_deref() == Some(model) {
        return false;
    }
    match doc.get_mut("model") {
        Some(item) => replace_value(item, Value::from(model)),
        None => {
            doc.insert("model", toml_edit::value(model));
        }
    }
    true
}

/// The servers under `mcp_servers`, or an error when that key is not a table.
pub(crate) fn read_servers(doc: &DocumentMut) -> Result<HashMap<String, McpServer>> {
    let Some(parent) = doc.get("mcp_servers") else {
        return Ok(HashMap::new());
    };
    let parent = parent
        .as_table_like()
        .ok_or_else(|| anyhow!("`mcp_servers` in config.toml is not a table"))?;
    Ok(parent
        .iter()
        .filter_map(|(name, item)| {
            let entry = item.as_table_like()?;
            Some((name.to_string(), decode(name, entry)))
        })
        .collect())
}

/// Merges `servers` into `mcp_servers` by name.
///
/// Servers only the file has are kept, and so are unmanaged keys inside an
/// updated server. An HTTP server with no `url`, or with headers Codex cannot
/// express (see [`split_headers`]), is skipped.
pub(crate) fn merge_servers(
    doc: &mut DocumentMut,
    servers: &HashMap<String, McpServer>,
) -> Result<WriteReport> {
    let mut report = WriteReport::default();
    let mut names: Vec<_> = servers.keys().collect();
    names.sort();
    let names: Vec<_> = names
        .into_iter()
        .filter(|name| {
            let server = &servers[*name];
            if server.transport != McpTransport::Http {
                return true;
            }
            if server.url.is_none() {
                // Neither `url` nor `command`: Codex cannot parse the entry.
                report.skipped.push(SkipReason::AgentSpecificFeature {
                    item: (*name).clone(),
                    feature: "HTTP server without a url".to_string(),
                    suggestion: "add a `url` to the server in the source config".to_string(),
                });
                return false;
            }
            let Err(header) = split_headers(server.headers.as_ref()) else {
                return true;
            };
            report.skipped.push(SkipReason::AgentSpecificFeature {
                item: (*name).clone(),
                feature: format!("header `{header}` with an environment variable inside its value"),
                suggestion:
                    "codex expands no ${VAR} inside a header; make the whole value ${VAR}, \
                             or `Bearer ${VAR}` for Authorization, or add this server by hand"
                        .to_string(),
            });
            false
        })
        .collect();
    if names.is_empty() {
        return Ok(report);
    }

    if doc.get("mcp_servers").is_none() {
        // Implicit: only `[mcp_servers.<name>]` headers are written, the
        // same form `skrills setup` uses.
        let mut parent = Table::new();
        parent.set_implicit(true);
        doc.insert("mcp_servers", Item::Table(parent));
    }
    let parent = doc.get_mut("mcp_servers").expect("inserted above");
    // A new server is spelled like its parent: inside `mcp_servers = { ... }`
    // it must be an inline table, and next to `mcp_servers.x.command = ...`
    // dotted keys read best.
    let parent_inline = matches!(parent, Item::Value(Value::InlineTable(_)));
    let parent_dotted = parent.as_table().is_some_and(Table::is_dotted);
    let parent = parent.as_table_like_mut().ok_or_else(|| {
        anyhow!("`mcp_servers` in config.toml is not a table; refusing to replace it")
    })?;

    for name in names {
        let server = &servers[name];
        match parent.get_mut(name) {
            Some(item) => {
                let entry = item.as_table_like_mut().ok_or_else(|| {
                    anyhow!("`mcp_servers.{name}` in config.toml is not a table; refusing to replace it")
                })?;
                if decode(name, entry) == normalized(server) && spelled_as(entry, &server.transport)
                {
                    report
                        .skipped
                        .push(SkipReason::Unchanged { item: name.clone() });
                    continue;
                }
                apply(entry, server);
            }
            None => {
                let mut entry = Table::new();
                apply(&mut entry, server);
                let item = if parent_inline {
                    Item::Value(Value::InlineTable(entry.into_inline_table()))
                } else {
                    entry.set_dotted(parent_dotted);
                    Item::Table(entry)
                };
                parent.insert(name, item);
            }
        }
        report.written += 1;
    }
    Ok(report)
}

/// Whether `entry` is already spelled for `transport`, so an otherwise equal
/// entry needs no rewrite. Codex ignores `type` and picks the transport from
/// `url` or `command`, so an entry without `type` is not rewritten just to
/// add it.
fn spelled_as(entry: &dyn TableLike, transport: &McpTransport) -> bool {
    let ty = entry.get("type").map(Item::as_str);
    match transport {
        McpTransport::Stdio => ty.is_none_or(|t| t == Some("stdio")),
        McpTransport::Http => {
            ty.is_none_or(|t| matches!(t, Some("http" | "streamable_http")))
                && !entry.contains_key("command")
        }
    }
}

/// `server` as [`decode`] would read it back after a write.
fn normalized(server: &McpServer) -> McpServer {
    let http = server.transport == McpTransport::Http;
    McpServer {
        name: server.name.clone(),
        transport: server.transport.clone(),
        command: if http {
            String::new()
        } else {
            server.command.clone()
        },
        args: if http {
            Vec::new()
        } else {
            server.args.clone()
        },
        env: if http {
            HashMap::new()
        } else {
            server.env.clone()
        },
        url: if http { server.url.clone() } else { None },
        headers: if http {
            split_headers(server.headers.as_ref())
                .ok()
                .and_then(join_headers)
        } else {
            None
        },
        enabled: server.enabled,
        allowed_tools: server.allowed_tools.clone(),
        disabled_tools: server.disabled_tools.clone(),
    }
}

/// One HTTP server's headers, in the three keys Codex reads them from.
#[derive(Debug, Default)]
struct CodexHeaders {
    /// `bearer_token_env_var`: from `Authorization: Bearer ${VAR}`.
    bearer_env: Option<String>,
    /// `env_http_headers`: headers whose whole value is `${VAR}`.
    from_env: HashMap<String, String>,
    /// `http_headers`: literal values.
    literal: HashMap<String, String>,
}

/// Sorts source headers into Codex's keys. Codex sends `http_headers`
/// verbatim, so a value that only partly references a variable has no
/// spelling; the first such header (by name) is the error.
fn split_headers(
    headers: Option<&HashMap<String, String>>,
) -> std::result::Result<CodexHeaders, String> {
    let mut out = CodexHeaders::default();
    let Some(headers) = headers else {
        return Ok(out);
    };
    let mut sorted: Vec<_> = headers.iter().collect();
    sorted.sort();
    for (name, value) in sorted {
        if !value.contains("${") {
            out.literal.insert(name.clone(), value.clone());
        } else if let Some(var) = env_reference(value) {
            out.from_env.insert(name.clone(), var.to_string());
        } else if let Some(var) = name
            .eq_ignore_ascii_case("authorization")
            .then(|| value.strip_prefix("Bearer "))
            .flatten()
            .and_then(env_reference)
        {
            out.bearer_env = Some(var.to_string());
        } else {
            return Err(name.clone());
        }
    }
    Ok(out)
}

/// `VAR` when `value` is exactly `${VAR}`.
fn env_reference(value: &str) -> Option<&str> {
    let var = value.strip_prefix("${")?.strip_suffix('}')?;
    let mut chars = var.chars();
    let first = chars.next()?;
    ((first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_'))
    .then_some(var)
}

/// The inverse of [`split_headers`], or `None` when there are no headers.
fn join_headers(split: CodexHeaders) -> Option<HashMap<String, String>> {
    let mut headers = split.literal;
    headers.extend(
        split
            .from_env
            .into_iter()
            .map(|(name, var)| (name, format!("${{{var}}}"))),
    );
    if let Some(var) = split.bearer_env {
        headers.insert("Authorization".to_string(), format!("Bearer ${{{var}}}"));
    }
    (!headers.is_empty()).then_some(headers)
}

/// Reads one server entry.
fn decode(name: &str, entry: &dyn TableLike) -> McpServer {
    let string = |key: &str| entry.get(key).and_then(Item::as_str).map(String::from);
    let strings = |key: &str| -> Vec<String> {
        entry
            .get(key)
            .and_then(Item::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let map = |key: &str| -> HashMap<String, String> {
        entry
            .get(key)
            .and_then(Item::as_table_like)
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.to_string(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let url = string("url");
    let headers = join_headers(CodexHeaders {
        bearer_env: string("bearer_token_env_var"),
        from_env: map("env_http_headers"),
        literal: map("http_headers"),
    });
    McpServer {
        name: name.to_string(),
        transport: if url.is_some() {
            McpTransport::Http
        } else {
            McpTransport::Stdio
        },
        command: string("command").unwrap_or_default(),
        args: strings("args"),
        env: map("env"),
        url,
        headers,
        enabled: entry.get("enabled").and_then(Item::as_bool).unwrap_or(true),
        allowed_tools: strings("enabled_tools"),
        disabled_tools: strings("disabled_tools"),
    }
}

/// Writes the keys sync owns for the server's transport into `entry`, in
/// place, and drops the other transport's keys: stdio owns `command`, `args`
/// and `env`, HTTP owns `url` and the header keys, and both own `type`,
/// `enabled`, `enabled_tools` and `disabled_tools`. Any other key
/// (`startup_timeout_sec`, ...) is left as the user wrote it.
///
/// An HTTP server's headers must already have passed [`split_headers`].
fn apply(entry: &mut dyn TableLike, server: &McpServer) {
    match server.transport {
        McpTransport::Stdio => {
            set(entry, "type", Some(Value::from("stdio")));
            set(entry, "command", Some(Value::from(server.command.as_str())));
            set(entry, "args", string_array(&server.args));
            set_map(entry, "env", &server.env);
            // HTTP keys left over from an earlier HTTP entry would make it both.
            for key in HTTP_KEYS {
                entry.remove(key);
            }
        }
        McpTransport::Http => {
            let headers = split_headers(server.headers.as_ref()).unwrap_or_default();
            set(entry, "type", Some(Value::from("http")));
            set(entry, "url", server.url.as_deref().map(Value::from));
            set(
                entry,
                "bearer_token_env_var",
                headers.bearer_env.as_deref().map(Value::from),
            );
            set_map(entry, "env_http_headers", &headers.from_env);
            set_map(entry, "http_headers", &headers.literal);
            for key in STDIO_KEYS {
                entry.remove(key);
            }
        }
    }
    set(
        entry,
        "enabled",
        (!server.enabled).then(|| Value::from(false)),
    );
    set(entry, "enabled_tools", string_array(&server.allowed_tools));
    set(
        entry,
        "disabled_tools",
        string_array(&server.disabled_tools),
    );
}

/// Keys that only make sense for a stdio server.
const STDIO_KEYS: &[&str] = &["command", "args", "env", "env_vars", "cwd"];

/// Keys that only make sense for an HTTP server.
const HTTP_KEYS: &[&str] = &[
    "url",
    "http_headers",
    "env_http_headers",
    "bearer_token_env_var",
];

fn string_array(items: &[String]) -> Option<Value> {
    (!items.is_empty()).then(|| Value::Array(items.iter().map(String::as_str).collect::<Array>()))
}

/// Sets or removes one key, keeping the old value's comment and spacing.
fn set(entry: &mut dyn TableLike, key: &str, value: Option<Value>) {
    match (entry.get_mut(key), value) {
        (Some(item), Some(value)) => replace_value(item, value),
        (None, Some(value)) => {
            entry.insert(key, Item::Value(value));
        }
        (Some(_), None) => {
            entry.remove(key);
        }
        (None, None) => {}
    }
}

/// Sets a string map. An existing table (`[mcp_servers.x.env]` or inline) is
/// updated key by key; a new one is written inline.
fn set_map(entry: &mut dyn TableLike, key: &str, map: &HashMap<String, String>) {
    if map.is_empty() {
        entry.remove(key);
        return;
    }
    let mut sorted: Vec<_> = map.iter().collect();
    sorted.sort();
    if let Some(existing) = entry.get_mut(key).and_then(Item::as_table_like_mut) {
        let stale: Vec<String> = existing
            .iter()
            .map(|(k, _)| k.to_string())
            .filter(|k| !map.contains_key(k))
            .collect();
        for k in stale {
            existing.remove(&k);
        }
        for (k, v) in sorted {
            set(existing, k, Some(Value::from(v.as_str())));
        }
        return;
    }
    let mut inline = InlineTable::new();
    for (k, v) in sorted {
        inline.insert(k, Value::from(v.as_str()));
    }
    set(entry, key, Some(Value::InlineTable(inline)));
}

/// Replaces the value in `item`, carrying over its decor (spacing and a
/// trailing comment). An equal scalar or string array is left untouched so
/// its original spelling survives.
fn replace_value(item: &mut Item, mut value: Value) {
    if let Some(old) = item.as_value() {
        if same_value(old, &value) {
            return;
        }
        *value.decor_mut() = old.decor().clone();
    }
    *item = Item::Value(value);
}

fn same_value(a: &Value, b: &Value) -> bool {
    fn strings(v: &Value) -> Option<Vec<&str>> {
        v.as_array()?.iter().map(Value::as_str).collect()
    }
    match (a, b) {
        (Value::String(x), Value::String(y)) => x.value() == y.value(),
        (Value::Boolean(x), Value::Boolean(y)) => x.value() == y.value(),
        (Value::Array(_), Value::Array(_)) => strings(a).is_some() && strings(a) == strings(b),
        _ => false,
    }
}
