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
    /// Markdown content of the command
    pub content: String,
    /// Original file path (for reference)
    pub source_path: PathBuf,
    /// Last modification time
    pub modified: SystemTime,
    /// SHA256 hash of content for change detection
    pub hash: String,
}

/// An MCP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpServer {
    /// Server name/identifier
    pub name: String,
    /// Command to execute (path to binary)
    pub command: String,
    /// Command arguments
    pub args: Vec<String>,
    /// Environment variables
    #[serde(default)]
    pub env: HashMap<String, String>,
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
}

/// Metadata about a sync operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMeta {
    pub source_agent: String,
    pub target_agent: String,
    pub synced_at: SystemTime,
}
