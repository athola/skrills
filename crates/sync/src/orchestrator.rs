//! Sync orchestrator that coordinates adapters and manages sync flow.

use crate::adapters::AgentAdapter;
use crate::report::{SkipReason, SyncReport, WriteReport};
use crate::Result;
use anyhow::bail;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Direction of sync operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncDirection {
    /// Sync from Claude to Codex
    ClaudeToCodex,
    /// Sync from Codex to Claude
    CodexToClaude,
}

/// Parameters for a sync operation.
///
/// ```
/// use skrills_sync::SyncParams;
///
/// let params = SyncParams { dry_run: true, ..Default::default() };
/// assert!(params.dry_run);
/// assert!(params.sync_skills);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncParams {
    /// Source agent name: "claude", "codex", or "auto"
    pub from: Option<String>,
    /// Perform dry run (preview only)
    pub dry_run: bool,
    /// Skip confirmation prompts
    pub force: bool,
    /// Sync skills
    #[serde(default = "default_true")]
    pub sync_skills: bool,
    /// Sync commands
    #[serde(default = "default_true")]
    pub sync_commands: bool,
    /// Skip overwriting existing commands on the target (only add new ones)
    #[serde(default)]
    pub skip_existing_commands: bool,
    /// Sync MCP servers
    #[serde(default = "default_true")]
    pub sync_mcp_servers: bool,
    /// Sync preferences
    #[serde(default = "default_true")]
    pub sync_preferences: bool,
    /// Include marketplace content (e.g. uninstalled plugins)
    #[serde(default)]
    pub include_marketplace: bool,
}

impl Default for SyncParams {
    fn default() -> Self {
        Self {
            from: None,
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: true,
            skip_existing_commands: false,
            sync_mcp_servers: true,
            sync_preferences: true,
            include_marketplace: false,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Orchestrates sync operations between agents.
pub struct SyncOrchestrator<S: AgentAdapter, T: AgentAdapter> {
    source: S,
    target: T,
}

impl<S: AgentAdapter, T: AgentAdapter> SyncOrchestrator<S, T> {
    /// Creates a new orchestrator with source and target adapters.
    pub fn new(source: S, target: T) -> Self {
        Self { source, target }
    }

    /// Returns the source adapter name.
    pub fn source_name(&self) -> &str {
        self.source.name()
    }

    /// Returns the target adapter name.
    pub fn target_name(&self) -> &str {
        self.target.name()
    }

    /// Performs the sync operation.
    pub fn sync(&self, params: &SyncParams) -> Result<SyncReport> {
        let mut report = SyncReport::new();

        // Sync commands
        if params.sync_commands {
            let commands = self.source.read_commands(params.include_marketplace)?;

            if params.force {
                // If force is true, we always write all commands, bypassing skip_existing_commands
                if params.dry_run {
                    report.commands.written = commands.len();
                } else {
                    report.commands = self.target.write_commands(&commands)?;
                }
            } else if params.dry_run {
                if params.skip_existing_commands {
                    let existing: HashSet<String> = self
                        .target
                        .read_commands(params.include_marketplace)?
                        .into_iter()
                        .map(|c| c.name)
                        .collect();

                    for cmd in &commands {
                        if existing.contains(&cmd.name) {
                            report.commands.skipped.push(SkipReason::WouldOverwrite {
                                item: cmd.name.clone(),
                            });
                        } else {
                            report.commands.written += 1;
                        }
                    }
                } else {
                    report.commands.written = commands.len();
                }
            } else if params.skip_existing_commands {
                let existing: HashSet<String> = self
                    .target
                    .read_commands(params.include_marketplace)?
                    .into_iter()
                    .map(|c| c.name)
                    .collect();

                let mut new_commands = Vec::new();
                let mut skipped = Vec::new();

                for cmd in commands {
                    if existing.contains(&cmd.name) {
                        skipped.push(SkipReason::WouldOverwrite {
                            item: cmd.name.clone(),
                        });
                    } else {
                        new_commands.push(cmd);
                    }
                }

                let mut cmd_report = if new_commands.is_empty() {
                    WriteReport::default()
                } else {
                    self.target.write_commands(&new_commands)?
                };

                cmd_report.skipped.extend(skipped);
                report.commands = cmd_report;
            } else {
                report.commands = self.target.write_commands(&commands)?;
            }
        }
        // Sync skills
        if params.sync_skills {
            let skills = self.source.read_skills()?;
            if !params.dry_run {
                report.skills = self.target.write_skills(&skills)?;
            } else {
                report.skills.written = skills.len();
            }
        }

        // Sync MCP servers
        if params.sync_mcp_servers {
            let servers = self.source.read_mcp_servers()?;
            if !params.dry_run {
                report.mcp_servers = self.target.write_mcp_servers(&servers)?;
            } else {
                report.mcp_servers.written = servers.len();
            }
        }

        // Sync preferences
        if params.sync_preferences {
            let prefs = self.source.read_preferences()?;
            if !params.dry_run {
                report.preferences = self.target.write_preferences(&prefs)?;
            } else {
                // Count non-empty preferences
                if prefs.model.is_some() {
                    report.preferences.written += 1;
                }
            }
        }

        report.success = true;
        report.summary = report.format_summary(self.source.name(), self.target.name());

        Ok(report)
    }
}

/// Determines sync direction from string input.
///
/// ```
/// use skrills_sync::{parse_direction, SyncDirection};
///
/// assert_eq!(parse_direction("claude").unwrap(), SyncDirection::ClaudeToCodex);
/// assert_eq!(parse_direction("codex").unwrap(), SyncDirection::CodexToClaude);
/// assert!(parse_direction("invalid").is_err());
/// ```
pub fn parse_direction(from: &str) -> Result<SyncDirection> {
    match from.to_lowercase().as_str() {
        "claude" => Ok(SyncDirection::ClaudeToCodex),
        "codex" => Ok(SyncDirection::CodexToClaude),
        _ => bail!("Unknown source '{}'. Use 'claude' or 'codex'", from),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{ClaudeAdapter, CodexAdapter};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn sync_commands_between_adapters() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Create source command
        let src_cmd_dir = src_dir.path().join("commands");
        fs::create_dir_all(&src_cmd_dir).unwrap();
        fs::write(src_cmd_dir.join("hello.md"), "# Hello").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.commands.written, 1);

        // Verify file was created
        let tgt_file = tgt_dir.path().join("prompts/hello.md");
        assert!(tgt_file.exists());
        assert_eq!(fs::read_to_string(&tgt_file).unwrap(), "# Hello");
    }

    #[test]
    fn skip_existing_commands_does_not_overwrite() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Source has a command with the same name as target
        let src_cmd_dir = src_dir.path().join("commands");
        fs::create_dir_all(&src_cmd_dir).unwrap();
        fs::write(src_cmd_dir.join("hello.md"), "# New Hello").unwrap();

        // Target already has the command
        let tgt_cmd_dir = tgt_dir.path().join("prompts");
        fs::create_dir_all(&tgt_cmd_dir).unwrap();
        fs::write(tgt_cmd_dir.join("hello.md"), "# Existing Hello").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            skip_existing_commands: true,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.commands.written, 0);
        assert_eq!(report.commands.skipped.len(), 1);

        // Ensure target file was not overwritten
        let tgt_file = tgt_dir.path().join("prompts/hello.md");
        assert_eq!(fs::read_to_string(&tgt_file).unwrap(), "# Existing Hello");
    }

    #[test]
    fn skip_existing_commands_still_writes_new_items() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        let src_cmd_dir = src_dir.path().join("commands");
        fs::create_dir_all(&src_cmd_dir).unwrap();
        fs::write(src_cmd_dir.join("hello.md"), "# New Hello").unwrap();
        fs::write(src_cmd_dir.join("greet.md"), "# Greet").unwrap();

        let tgt_cmd_dir = tgt_dir.path().join("prompts");
        fs::create_dir_all(&tgt_cmd_dir).unwrap();
        fs::write(tgt_cmd_dir.join("hello.md"), "# Existing Hello").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            skip_existing_commands: true,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.commands.written, 1);
        assert_eq!(report.commands.skipped.len(), 1);

        // New command should be written, existing remains unchanged
        let hello_path = tgt_dir.path().join("prompts/hello.md");
        let greet_path = tgt_dir.path().join("prompts/greet.md");
        assert_eq!(fs::read_to_string(&hello_path).unwrap(), "# Existing Hello");
        assert_eq!(fs::read_to_string(&greet_path).unwrap(), "# Greet");
    }

    #[test]
    fn dry_run_does_not_write() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        let src_cmd_dir = src_dir.path().join("commands");
        fs::create_dir_all(&src_cmd_dir).unwrap();
        fs::write(src_cmd_dir.join("hello.md"), "# Hello").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            dry_run: true,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.commands.written, 1);

        // Verify nothing was actually written
        let tgt_file = tgt_dir.path().join("prompts/hello.md");
        assert!(!tgt_file.exists());
    }

    #[test]
    fn sync_mcp_servers() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Create source MCP config
        let settings_path = src_dir.path().join("settings.json");
        fs::write(
            &settings_path,
            r#"{
            "mcpServers": {
                "test-server": {
                    "command": "/usr/bin/test"
                }
            }
        }"#,
        )
        .unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: true,
            sync_preferences: false,
            sync_skills: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.mcp_servers.written, 1);

        // Verify config was created
        let tgt_config = tgt_dir.path().join("config.json");
        assert!(tgt_config.exists());
    }

    #[test]
    fn parse_direction_claude() {
        let dir = parse_direction("claude").unwrap();
        assert_eq!(dir, SyncDirection::ClaudeToCodex);
    }

    #[test]
    fn parse_direction_codex() {
        let dir = parse_direction("codex").unwrap();
        assert_eq!(dir, SyncDirection::CodexToClaude);
    }

    #[test]
    fn parse_direction_invalid() {
        let result = parse_direction("invalid");
        assert!(result.is_err());
    }

    #[test]
    fn orchestrator_names() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        assert_eq!(orchestrator.source_name(), "claude");
        assert_eq!(orchestrator.target_name(), "codex");
    }
}
