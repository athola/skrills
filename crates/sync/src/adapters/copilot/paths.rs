//! Path resolution utilities for Copilot adapter.

use crate::Result;
use anyhow::Context;
use std::path::PathBuf;

/// Resolves the configuration root following XDG Base Directory Specification.
pub fn resolve_config_root() -> Result<PathBuf> {
    // First, try XDG-compliant path: $XDG_CONFIG_HOME/copilot or ~/.config/copilot
    if let Some(config_dir) = dirs::config_dir() {
        let xdg_path = config_dir.join("copilot");
        // Use XDG path if it exists OR if no legacy path exists
        // (prefer XDG for new installations)
        let home = dirs::home_dir();
        let legacy_path = home.as_ref().map(|h| h.join(".copilot"));

        if xdg_path.exists() {
            return Ok(xdg_path);
        }

        // If legacy path doesn't exist either, prefer XDG for new installations
        if legacy_path.as_ref().is_none_or(|p| !p.exists()) {
            return Ok(xdg_path);
        }
    }

    // Fallback to legacy ~/.copilot
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".copilot"))
}

/// Returns the skills directory path.
pub fn skills_dir(root: &std::path::Path) -> PathBuf {
    root.join("skills")
}

/// Returns the agents directory path.
pub fn agents_dir(root: &std::path::Path) -> PathBuf {
    root.join("agents")
}

/// Returns the prompts directory path.
pub fn prompts_dir(root: &std::path::Path) -> PathBuf {
    root.join("prompts")
}

/// Returns the instructions directory path.
pub fn instructions_dir(root: &std::path::Path) -> PathBuf {
    root.join("instructions")
}

/// Path to MCP server configuration (separate from main config).
pub fn mcp_config_path(root: &std::path::Path) -> PathBuf {
    root.join("mcp-config.json")
}

/// Path to preferences/settings (model, security fields).
pub fn config_path(root: &std::path::Path) -> PathBuf {
    root.join("config.json")
}
