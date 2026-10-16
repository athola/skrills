//! Path resolution utilities for Copilot adapter.

use crate::Result;
use anyhow::Context;
use std::path::{Path, PathBuf};

/// Resolves the directory Copilot CLI reads: `COPILOT_HOME`, else
/// `~/.copilot` (docs.github.com/en/copilot/reference/copilot-cli-reference/cli-config-dir-reference).
///
/// An existing XDG `copilot` directory is kept so earlier syncs are not
/// orphaned. On macOS only an explicit `XDG_CONFIG_HOME` counts:
/// `dirs::config_dir()` is `~/Library/Application Support` there, which
/// Copilot never reads.
pub fn resolve_config_root() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    let non_empty = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    let xdg_dir = match non_empty("XDG_CONFIG_HOME") {
        Some(dir) => Some(PathBuf::from(dir)),
        None if cfg!(target_os = "macos") => None,
        None => dirs::config_dir(),
    }
    .map(|dir| dir.join("copilot"));
    Ok(choose_config_root(
        non_empty("COPILOT_HOME").map(PathBuf::from),
        xdg_dir,
        &home,
    ))
}

/// The lookup [`resolve_config_root`] applies, without reading the environment.
pub(crate) fn choose_config_root(
    copilot_home: Option<PathBuf>,
    xdg_dir: Option<PathBuf>,
    home: &Path,
) -> PathBuf {
    if let Some(dir) = copilot_home {
        return dir;
    }
    match xdg_dir {
        Some(dir) if dir.exists() => dir,
        _ => home.join(".copilot"),
    }
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
