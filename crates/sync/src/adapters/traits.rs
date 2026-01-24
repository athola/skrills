//! Trait definition for agent adapters.

use crate::common::{Command, CommonConfig, McpServer, Preferences};
use crate::report::WriteReport;
use crate::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Describes which fields an adapter supports for sync.
#[derive(Debug, Clone, Default)]
pub struct FieldSupport {
    pub commands: bool,
    pub mcp_servers: bool,
    pub preferences: bool,
    pub skills: bool,
    pub hooks: bool,
    pub agents: bool,
    pub instructions: bool,
}

#[cfg(test)]
use mockall::automock;

/// Adapter for reading and writing configuration for a specific agent.
#[cfg_attr(test, automock)]
pub trait AgentAdapter: Send + Sync {
    /// Agent identifier (e.g., "claude", "codex")
    fn name(&self) -> &str;

    /// Root configuration directory (e.g., ~/.claude, ~/.codex)
    fn config_root(&self) -> PathBuf;

    /// What this adapter supports
    fn supported_fields(&self) -> FieldSupport;

    // --- Read operations ---

    /// Read slash commands from native format
    fn read_commands(&self, include_marketplace: bool) -> Result<Vec<Command>>;

    /// Read MCP server configurations from native format
    fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>>;

    /// Read preferences from native format
    fn read_preferences(&self) -> Result<Preferences>;

    /// Read skills from native format
    fn read_skills(&self) -> Result<Vec<Command>>;

    /// Read hooks from native format
    fn read_hooks(&self) -> Result<Vec<Command>>;

    /// Read agents from native format
    fn read_agents(&self) -> Result<Vec<Command>>;

    /// Read instructions from native format (e.g., CLAUDE.md, *.instructions.md)
    fn read_instructions(&self) -> Result<Vec<Command>>;

    /// Read complete configuration
    fn read_all(&self) -> Result<CommonConfig> {
        Ok(CommonConfig {
            commands: self.read_commands(false)?,
            mcp_servers: self.read_mcp_servers()?,
            preferences: self.read_preferences()?,
            skills: self.read_skills()?,
            hooks: self.read_hooks()?,
            agents: self.read_agents()?,
            instructions: self.read_instructions()?,
        })
    }

    // --- Write operations ---

    /// Write commands to native format
    fn write_commands(&self, commands: &[Command]) -> Result<WriteReport>;

    /// Write MCP servers to native format
    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport>;

    /// Write preferences to native format
    fn write_preferences(&self, prefs: &Preferences) -> Result<WriteReport>;

    /// Write skills to native format
    fn write_skills(&self, skills: &[Command]) -> Result<WriteReport>;

    /// Write hooks to native format
    fn write_hooks(&self, hooks: &[Command]) -> Result<WriteReport>;

    /// Write agents to native format
    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport>;

    /// Write instructions to native format (e.g., CLAUDE.md, *.instructions.md)
    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport>;
}
