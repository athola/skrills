//! Cursor adapter for reading/writing ~/.cursor configuration.
//!
//! ## Architecture
//!
//! Follows the modular pattern established by the Copilot adapter:
//! one sub-module per artifact type, coordinated by this module's
//! `AgentAdapter` trait implementation.
//!
//! ## Key differences from other adapters:
//!
//! - **Rules**: Cursor uses `.mdc` files with frontmatter (`description`, `globs`,
//!   `alwaysApply`) in `.cursor/rules/`. No other adapter writes this format.
//! - **Hooks**: Cursor supports 18+ lifecycle events (camelCase) vs Claude's 8
//!   (PascalCase). Event name mapping is handled in the hooks module.
//! - **Skills**: Cursor skills have no frontmatter — Claude frontmatter is stripped
//!   on write and absent on read. **This is lossy**: a Claude→Cursor→Claude
//!   roundtrip loses all frontmatter metadata (name, description, dependencies).
//! - **Agents**: Field translation: `background` ↔ `is_background`, model name
//!   mapping, `tools`/`isolation` dropped (Cursor-only: `readonly`).
//! - **Commands**: Near-identical to Claude format (`.cursor/commands/*.md`).
//! - **MCP**: `.cursor/mcp.json` (similar to Claude's `.mcp.json`).

mod agents;
mod commands;
mod hooks;
mod mcp;
mod paths;
mod rules;
mod skills;
pub(crate) mod utils;

#[cfg(test)]
mod tests;

use super::traits::{AgentAdapter, FieldSupport};
use crate::common::{Command, McpServer, Preferences};
use crate::report::WriteReport;
use crate::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Adapter for Cursor IDE configuration.
#[derive(Debug)]
pub struct CursorAdapter {
    root: PathBuf,
}

impl CursorAdapter {
    /// Creates a new CursorAdapter with the default root (~/.cursor).
    pub fn new() -> Result<Self> {
        let root = paths::resolve_config_root()?;
        Ok(Self { root })
    }

    /// Creates a CursorAdapter with a custom root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self { root }
    }
}

impl AgentAdapter for CursorAdapter {
    fn name(&self) -> &str {
        "cursor"
    }

    fn config_root(&self) -> PathBuf {
        self.root.clone()
    }

    fn supported_fields(&self) -> FieldSupport {
        FieldSupport {
            commands: true,
            mcp_servers: true,
            preferences: false, // Cursor preferences are not yet mapped
            skills: true,
            hooks: true,
            agents: true,
            instructions: true, // Rules (.mdc) mapped via instructions
        }
    }

    fn read_commands(&self, _include_marketplace: bool) -> Result<Vec<Command>> {
        commands::read_commands(&self.root)
    }

    fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>> {
        mcp::read_mcp_servers(&self.root)
    }

    fn read_preferences(&self) -> Result<Preferences> {
        Ok(Preferences::default())
    }

    fn read_skills(&self) -> Result<Vec<Command>> {
        skills::read_skills(&self.root)
    }

    fn read_hooks(&self) -> Result<Vec<Command>> {
        hooks::read_hooks(&self.root)
    }

    fn read_agents(&self) -> Result<Vec<Command>> {
        agents::read_agents(&self.root)
    }

    fn read_instructions(&self) -> Result<Vec<Command>> {
        rules::read_rules(&self.root)
    }

    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
        commands::write_commands(&self.root, commands)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        mcp::write_mcp_servers(&self.root, servers)
    }

    fn write_preferences(&self, _prefs: &Preferences) -> Result<WriteReport> {
        let mut report = WriteReport::default();
        report
            .skipped
            .push(crate::report::SkipReason::AgentSpecificFeature {
                item: "preferences".to_string(),
                feature: "Cursor preferences mapping not yet implemented".to_string(),
                suggestion: "Preferences sync is not supported for Cursor".to_string(),
            });
        Ok(report)
    }

    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
        skills::write_skills(&self.root, skills)
    }

    fn write_hooks(&self, hooks: &[Command]) -> Result<WriteReport> {
        hooks::write_hooks(&self.root, hooks)
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        agents::write_agents(&self.root, agents)
    }

    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport> {
        rules::write_rules(&self.root, instructions)
    }
}
