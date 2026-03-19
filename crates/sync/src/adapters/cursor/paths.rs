//! Path resolution utilities for Cursor adapter.

use crate::Result;
use anyhow::Context;
use std::path::PathBuf;

/// Resolves the Cursor configuration root.
///
/// Cursor uses `~/.cursor/` directly (no XDG resolution needed).
pub fn resolve_config_root() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".cursor"))
}

/// Returns the skills directory path.
pub fn skills_dir(root: &std::path::Path) -> PathBuf {
    root.join("skills")
}

/// Returns the commands directory path.
pub fn commands_dir(root: &std::path::Path) -> PathBuf {
    root.join("commands")
}

/// Returns the agents directory path.
pub fn agents_dir(root: &std::path::Path) -> PathBuf {
    root.join("agents")
}

/// Returns the rules directory path (Cursor-specific `.mdc` files).
pub fn rules_dir(root: &std::path::Path) -> PathBuf {
    root.join("rules")
}

/// Path to hooks configuration.
pub fn hooks_path(root: &std::path::Path) -> PathBuf {
    root.join("hooks.json")
}

/// Path to MCP server configuration.
pub fn mcp_config_path(root: &std::path::Path) -> PathBuf {
    root.join("mcp.json")
}
