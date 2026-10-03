//! MCP Servers API endpoints.
//!
//! REST API for discovering MCP server configurations across all CLI adapters.
//!
//! ## Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/api/mcp-servers` | List MCP servers from all adapters |
//!
//! Configured servers routinely carry credentials (API keys in `env`, tokens
//! in `args` or the URL). The response keeps the shape of each entry but
//! replaces every environment value and anything that looks like a secret in
//! `args` or `url` with [`REDACTED`].

use axum::{http::StatusCode, routing::get, Json, Router};
use serde::Serialize;
use skrills_sync::adapters::traits::AgentAdapter;
use skrills_sync::common::{McpServer, McpTransport};
use skrills_sync::{ClaudeAdapter, CodexAdapter, CopilotAdapter, CursorAdapter};
use std::collections::HashMap;

/// Placeholder that replaces credential values in API responses.
const REDACTED: &str = "[redacted]";

/// Fragments that mark a flag or variable name as carrying a secret.
const SECRET_MARKERS: &[&str] = &[
    "token",
    "key",
    "secret",
    "password",
    "passwd",
    "pwd",
    "auth",
    "credential",
    "bearer",
    "cookie",
    "session",
];

/// MCP server info for API response.
#[derive(Debug, Serialize)]
pub struct McpServerResponse {
    pub name: String,
    pub source: String,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    /// Environment variable names; every value is redacted.
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

fn looks_secret(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SECRET_MARKERS.iter().any(|m| lower.contains(m))
}

/// Redact secret-looking values in a command line.
///
/// Handles `--flag value`, `--flag=value`, `NAME=value`, and header-style
/// arguments such as `Authorization: Bearer ...`.
fn redact_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut redact_next = false;
    for arg in args {
        if redact_next {
            out.push(REDACTED.to_string());
            redact_next = false;
            continue;
        }
        if let Some((name, _)) = arg.split_once('=') {
            if looks_secret(name) {
                out.push(format!("{name}={REDACTED}"));
                continue;
            }
        }
        if let Some((name, _)) = arg.split_once(':') {
            if !arg.contains("://") && looks_secret(name) {
                out.push(format!("{name}: {REDACTED}"));
                continue;
            }
        }
        if arg.starts_with('-') && !arg.contains('=') && looks_secret(arg) {
            redact_next = true;
        }
        out.push(arg.clone());
    }
    out
}

/// Strip credentials from a URL: user info, and the value of every query
/// parameter. An unparseable URL is withheld entirely.
fn redact_url(raw: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(raw) else {
        return REDACTED.to_string();
    };
    if !url.username().is_empty() || url.password().is_some() {
        // Both setters only fail for URLs that cannot carry user info, in
        // which case there is nothing to strip.
        let _ = url.set_username("");
        let _ = url.set_password(None);
    }
    if url.query().is_some() {
        let keys: Vec<String> = url.query_pairs().map(|(k, _)| k.into_owned()).collect();
        url.query_pairs_mut()
            .clear()
            .extend_pairs(keys.iter().map(|k| (k.as_str(), REDACTED)));
    }
    url.to_string()
}

/// Build the API view of one configured server with credentials removed.
fn to_response(name: String, source: &str, server: McpServer) -> McpServerResponse {
    McpServerResponse {
        name,
        source: source.to_string(),
        transport: match server.transport {
            McpTransport::Stdio => "stdio".to_string(),
            McpTransport::Http => "http".to_string(),
        },
        command: server.command,
        args: redact_args(&server.args),
        env: server
            .env
            .into_keys()
            .map(|k| (k, REDACTED.to_string()))
            .collect(),
        url: server.url.as_deref().map(redact_url),
        enabled: server.enabled,
        allowed_tools: server.allowed_tools,
        disabled_tools: server.disabled_tools,
    }
}

/// Read MCP servers from a single adapter, tagging each with the source name.
fn collect_from_adapter(
    adapter: &dyn AgentAdapter,
    source: &str,
    out: &mut Vec<McpServerResponse>,
) {
    match adapter.read_mcp_servers() {
        Ok(servers) => {
            for (name, server) in servers {
                out.push(to_response(name, source, server));
            }
        }
        Err(e) => {
            tracing::warn!(source, error = %e, "could not read MCP server configuration");
        }
    }
}

fn adapter_unavailable(source: &str, error: &dyn std::fmt::Display) {
    tracing::warn!(source, error = %error, "MCP server adapter unavailable");
}

/// Read every adapter's configuration. Blocking file I/O.
fn collect_all_servers() -> McpServersListResponse {
    let mut servers = Vec::new();

    match ClaudeAdapter::new() {
        Ok(adapter) => collect_from_adapter(&adapter, "claude", &mut servers),
        Err(e) => adapter_unavailable("claude", &e),
    }
    match CodexAdapter::new() {
        Ok(adapter) => collect_from_adapter(&adapter, "codex", &mut servers),
        Err(e) => adapter_unavailable("codex", &e),
    }
    match CopilotAdapter::new() {
        Ok(adapter) => collect_from_adapter(&adapter, "copilot", &mut servers),
        Err(e) => adapter_unavailable("copilot", &e),
    }
    match CursorAdapter::new() {
        Ok(adapter) => collect_from_adapter(&adapter, "cursor", &mut servers),
        Err(e) => adapter_unavailable("cursor", &e),
    }

    servers.sort_by(|a, b| a.source.cmp(&b.source).then(a.name.cmp(&b.name)));
    let total = servers.len();
    McpServersListResponse { servers, total }
}

/// List MCP servers from all adapters.
async fn list_mcp_servers() -> Result<Json<McpServersListResponse>, StatusCode> {
    tokio::task::spawn_blocking(collect_all_servers)
        .await
        .map(Json)
        .map_err(|e| {
            tracing::warn!(error = %e, "MCP server listing task panicked");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// Create MCP servers API routes.
pub fn mcp_servers_routes() -> Router {
    Router::new().route("/api/mcp-servers", get(list_mcp_servers))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn env_values_are_redacted_but_names_kept() {
        let server: McpServer = serde_json::from_value(serde_json::json!({
            "name": "gh",
            "command": "npx",
            "env": {"GITHUB_TOKEN": "ghp_live_secret", "LOG_LEVEL": "debug"},
        }))
        .unwrap();
        let resp = to_response("gh".into(), "claude", server);
        let body = serde_json::to_string(&resp).unwrap();
        assert!(!body.contains("ghp_live_secret"), "{body}");
        assert!(!body.contains("debug"), "{body}");
        assert_eq!(
            resp.env.get("GITHUB_TOKEN").map(String::as_str),
            Some(REDACTED)
        );
        assert!(resp.env.contains_key("LOG_LEVEL"));
    }

    #[test]
    fn secret_args_are_redacted() {
        let args = strings(&[
            "-y",
            "@modelcontextprotocol/server-github",
            "--api-key",
            "sk-live-1",
            "--token=tok-2",
            "OPENAI_API_KEY=sk-3",
            "Authorization: Bearer tok-4",
            "--port",
            "8080",
            "https://example.com/mcp",
        ]);
        let out = redact_args(&args);
        let joined = out.join(" ");
        for secret in ["sk-live-1", "tok-2", "sk-3", "tok-4"] {
            assert!(!joined.contains(secret), "{secret} leaked: {joined}");
        }
        assert!(joined.contains("--port 8080"), "{joined}");
        assert!(joined.contains("https://example.com/mcp"), "{joined}");
        assert!(joined.contains("@modelcontextprotocol/server-github"));
    }

    #[test]
    fn url_credentials_and_query_values_are_redacted() {
        let out = redact_url("https://user:pass@api.example.com/mcp?api_key=abc&mode=x");
        assert!(!out.contains("user"), "{out}");
        assert!(!out.contains("pass"), "{out}");
        assert!(!out.contains("abc"), "{out}");
        assert!(out.starts_with("https://api.example.com/mcp?"), "{out}");
        assert!(out.contains("api_key="), "{out}");

        assert_eq!(
            redact_url("https://example.com/mcp"),
            "https://example.com/mcp"
        );
        assert_eq!(redact_url("not a url with secret"), REDACTED);
    }
}
