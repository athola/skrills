//! Common schema types for cross-agent configuration sync.

#![allow(dead_code)] // Types will be used by adapters in subsequent tasks

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// A slash command that can be synced between agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Command {
    /// Command name without leading slash (e.g., "commit-msg")
    pub name: String,
    /// Raw Markdown content (bytes) of the command. Stored as bytes so we
    /// don't choke on non-UTF-8 files (common in multilingual skills).
    pub content: Vec<u8>,
    /// Original file path (for reference)
    pub source_path: PathBuf,
    /// Last modification time
    pub modified: SystemTime,
    /// SHA256 hash of content for change detection
    pub hash: String,
}

/// Transport type for MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum McpTransport {
    /// stdio transport (command execution)
    #[default]
    Stdio,
    /// HTTP transport (SSE/HTTP-based MCP)
    Http,
}

/// An MCP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServer {
    /// Server name/identifier
    pub name: String,
    /// Transport type (stdio or http)
    #[serde(default)]
    pub transport: McpTransport,
    /// Command to execute (for stdio transport)
    #[serde(default)]
    pub command: String,
    /// Command arguments (for stdio transport)
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables (for stdio transport)
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// URL for HTTP transport
    #[serde(default)]
    pub url: Option<String>,
    /// HTTP headers (for HTTP transport)
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    /// Whether the server is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Agent-agnostic preferences that can be synced.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preferences {
    /// Preferred model (if set)
    pub model: Option<String>,
    /// Agent-specific fields that don't map cleanly
    #[serde(default)]
    pub custom: HashMap<String, serde_json::Value>,
}

/// Complete syncable configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommonConfig {
    pub commands: Vec<Command>,
    pub mcp_servers: HashMap<String, McpServer>,
    pub preferences: Preferences,
    pub skills: Vec<Command>,
    pub hooks: Vec<Command>,
    pub agents: Vec<Command>,
}

/// Metadata about a sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMeta {
    pub source_agent: String,
    pub target_agent: String,
    pub synced_at: SystemTime,
}
