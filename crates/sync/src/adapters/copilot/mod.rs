//! Copilot adapter for reading/writing Copilot CLI configuration.
//!
//! ## Path Resolution (XDG-compliant)
//!
//! The adapter follows the XDG Base Directory Specification:
//! 1. If `XDG_CONFIG_HOME` is set → `$XDG_CONFIG_HOME/copilot`
//! 2. If unset → `~/.config/copilot` (XDG default)
//! 3. Fallback → `~/.copilot` (legacy location)
//!
//! ## Key differences from Codex:
//! - MCP servers: Stored in `mcp-config.json` (NOT in `config.json`)
//! - Preferences: Stored in `config.json`, must preserve security fields
//! - Skills: Same format as Codex (`skills/<name>/SKILL.md`)
//! - Commands: NOT synced - Copilot prompts (detailed instruction files) are
//!   conceptually different from Claude commands/Codex prompts (quick atomic shortcuts)
//! - No config.toml feature flag management

mod agents;
mod commands;
mod mcp;
mod paths;
mod preferences;
mod skills;
mod utils;

#[cfg(test)]
mod tests;

use super::traits::{AgentAdapter, FieldSupport};
use crate::common::{Command, McpServer, Preferences};
use crate::report::WriteReport;
use crate::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Adapter for GitHub Copilot CLI configuration.
pub struct CopilotAdapter {
    root: PathBuf,
}

impl CopilotAdapter {
    /// Creates a new CopilotAdapter using XDG-compliant path resolution.
    ///
    /// Path precedence:
    /// 1. `$XDG_CONFIG_HOME/copilot` (if XDG_CONFIG_HOME is set)
    /// 2. `~/.config/copilot` (XDG default)
    /// 3. `~/.copilot` (legacy fallback)
    pub fn new() -> Result<Self> {
        let root = paths::resolve_config_root()?;
        Ok(Self { root })
    }

    /// Creates a CopilotAdapter with a custom root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// Resolves the configuration root following XDG Base Directory Specification.
    /// Exposed for testing.
    #[cfg(test)]
    fn resolve_config_root() -> Result<PathBuf> {
        paths::resolve_config_root()
    }
}

// Note: We intentionally do not implement Default for CopilotAdapter because
// construction requires home directory resolution which can fail. Use
// CopilotAdapter::new() or CopilotAdapter::with_root() instead.

impl AgentAdapter for CopilotAdapter {
    fn name(&self) -> &str {
        "copilot"
    }

    fn config_root(&self) -> PathBuf {
        self.root.clone()
    }

    fn supported_fields(&self) -> FieldSupport {
        FieldSupport {
            commands: false, // Copilot prompts are NOT equivalent to Claude commands/Codex prompts
            mcp_servers: true,
            preferences: true,
            skills: true,
            hooks: false,       // Copilot doesn't support hooks
            agents: true,       // Copilot supports custom agents in ~/.copilot/agents/
            instructions: true, // Copilot supports *.instructions.md files
        }
    }

    fn read_commands(&self, include_marketplace: bool) -> Result<Vec<Command>> {
        commands::read_commands(&self.root, include_marketplace)
    }

    fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>> {
        mcp::read_mcp_servers(&self.root)
    }

    fn read_preferences(&self) -> Result<Preferences> {
        preferences::read_preferences(&self.root)
    }

    fn read_skills(&self) -> Result<Vec<Command>> {
        skills::read_skills(&self.root)
    }

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        commands::write_commands(&self.root, commands)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        mcp::write_mcp_servers(&self.root, servers)
    }

    fn write_preferences(&self, prefs: &Preferences) -> Result<WriteReport> {
        preferences::write_preferences(&self.root, prefs)
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        skills::write_skills(&self.root, skills)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        agents::read_hooks(&self.root)
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        agents::read_agents(&self.root)
    }

    fn write_hooks(&self, hooks: &[Command]) -> Result<WriteReport> {
        agents::write_hooks(&self.root, hooks)
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        agents::write_agents(&self.root, agents)
    }

    fn read_instructions(&self) -> Result<Vec<Command>> {
        agents::read_instructions(&self.root)
    }

    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport> {
        agents::write_instructions(&self.root, instructions)
    }
}
