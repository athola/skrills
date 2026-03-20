//! Common schema types for cross-agent configuration sync.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// A companion file within a skill directory (e.g., helpers, sub-modules).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModuleFile {
    /// Relative path within the skill directory (e.g., "helpers.md", "sub/module.py")
    pub relative_path: PathBuf,
    /// File content as bytes
    pub content: Vec<u8>,
    /// SHA256 hash for change detection
    pub hash: String,
}

/// Describes the format of a [`Command`]'s content bytes.
///
/// Used to avoid ambiguous parse-and-fallback when content could be either
/// structured data (JSON) or plain text (markdown/shell commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContentFormat {
    /// Markdown or plain text (default for commands, skills, instructions).
    #[default]
    Markdown,
    /// JSON-structured content (e.g., hook entries serialized as a JSON array).
    Json,
}

/// A slash command that can be synced between agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Command {
    /// Command name without leading slash (e.g., "commit-msg")
    pub name: String,
    /// Raw content bytes. Stored as bytes so we don't choke on non-UTF-8
    /// files (common in multilingual skills).
    pub content: Vec<u8>,
    /// Original file path (for reference)
    pub source_path: PathBuf,
    /// Last modification time
    pub modified: SystemTime,
    /// SHA256 hash of content for change detection
    pub hash: String,
    /// Companion files for modular skills (e.g., helpers, sub-modules).
    /// Empty for commands; populated for directory-based skills.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<ModuleFile>,
    /// Format of the content bytes. Defaults to [`ContentFormat::Markdown`].
    /// Hook adapters should set this to [`ContentFormat::Json`] when the
    /// content is a serialized JSON array of hook entries.
    #[serde(default)]
    pub content_format: ContentFormat,
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
    pub instructions: Vec<Command>,
}
