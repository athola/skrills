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
use crate::common::{Command, McpServer, PluginAsset, Preferences};
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
            instructions: true,  // Rules (.mdc) mapped via instructions
            plugin_assets: true, // Cursor mirrors Claude's plugin cache
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

    fn write_plugin_assets(&self, assets: &[PluginAsset]) -> Result<WriteReport> {
        use crate::report::SkipReason;
        use std::fs;
        use tracing::debug;

        let mut report = WriteReport::default();

        if assets.is_empty() {
            return Ok(report);
        }

        let cache_dir = self.root.join("plugins").join("cache");
        // Create cache dir upfront so is_path_contained can canonicalize it
        fs::create_dir_all(&cache_dir)?;

        for asset in assets {
            // Mirror Claude's cache structure: plugins/cache/<publisher>/<plugin>/<version>/
            let target_dir = cache_dir
                .join(&asset.publisher)
                .join(&asset.plugin_name)
                .join(&asset.version);
            let target_path = target_dir.join(&asset.relative_path);

            // Path containment check
            if !crate::adapters::utils::is_path_contained(&target_path, &cache_dir) {
                debug!(
                    path = %asset.relative_path.display(),
                    "Skipping plugin asset with path outside cache directory"
                );
                continue;
            }

            // Check if unchanged
            if target_path.exists() {
                let existing = fs::read(&target_path).unwrap_or_default();
                if crate::adapters::utils::hash_content(&existing) == asset.hash {
                    report.skipped.push(SkipReason::Unchanged {
                        item: format!(
                            "{}/{}:{}",
                            asset.plugin_name,
                            asset.version,
                            asset.relative_path.display()
                        ),
                    });
                    continue;
                }
            }

            // Ensure parent directory exists
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }

            // Write the file
            fs::write(&target_path, &asset.content)?;

            // Preserve executable permissions
            #[cfg(unix)]
            if asset.executable {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o755);
                fs::set_permissions(&target_path, perms)?;
            }

            debug!(
                plugin = %asset.plugin_name,
                path = %asset.relative_path.display(),
                "Wrote plugin asset to Cursor"
            );
            report.written += 1;
        }

        Ok(report)
    }
}
