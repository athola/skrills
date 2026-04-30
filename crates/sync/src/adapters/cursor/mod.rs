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
//! - **Skills**: Cursor skills have no YAML frontmatter — Claude frontmatter is stripped
//!   on write, but `description` is preserved as a plain-text first line and `model_hint`
//!   as an HTML comment. **Partially lossy**: a Claude→Cursor→Claude roundtrip loses
//!   most frontmatter metadata (name, dependencies, version, tags).
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
    kill_switch: Option<skrills_snapshot::KillSwitch>,
}

impl CursorAdapter {
    /// Creates a new CursorAdapter with the default root (~/.cursor).
    pub fn new() -> Result<Self> {
        let root = paths::resolve_config_root()?;
        Ok(Self {
            root,
            kill_switch: None,
        })
    }

    /// Creates a CursorAdapter with a custom root (for testing).
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            root,
            kill_switch: None,
        }
    }

    /// Attach a [`KillSwitch`](skrills_snapshot::KillSwitch) so that mutating
    /// operations refuse with [`SyncError::TokenBudgetExceeded`](crate::SyncError)
    /// when the cold-window engine has engaged it (FR12).
    #[must_use]
    pub fn with_kill_switch(mut self, switch: skrills_snapshot::KillSwitch) -> Self {
        self.kill_switch = Some(switch);
        self
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
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        commands::write_commands(&self.root, commands)
    }

    fn write_mcp_servers(&self, servers: &HashMap<String, McpServer>) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        mcp::write_mcp_servers(&self.root, servers)
    }

    fn write_preferences(&self, _prefs: &Preferences) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
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
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        skills::write_skills(&self.root, skills)
    }

    fn write_hooks(&self, hooks: &[Command]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        hooks::write_hooks(&self.root, hooks)
    }

    fn write_agents(&self, agents: &[Command]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        agents::write_agents(&self.root, agents)
    }

    fn write_instructions(&self, instructions: &[Command]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        rules::write_rules(&self.root, instructions)
    }

    /// Writes plugin manifests to `~/.cursor/plugins/local/<plugin>/.cursor-plugin/plugin.json`.
    ///
    /// Only writes the manifest files — Cursor discovers actual plugin content
    /// (skills, agents, hooks) from `~/.claude/plugins/cache/` natively.
    /// The local manifests exist solely so `/plugins` shows installed plugins.
    fn write_plugin_assets(&self, assets: &[PluginAsset]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        use crate::adapters::utils::sanitize_name;
        use crate::report::SkipReason;
        use std::collections::HashSet;
        use std::fs;
        use tracing::debug;

        let mut report = WriteReport::default();

        if assets.is_empty() {
            return Ok(report);
        }

        let local_dir = self.root.join("plugins").join("local");
        fs::create_dir_all(&local_dir)?;

        // Track all referenced plugins so pruning never removes an active one,
        // regardless of whether the batch contains their manifest.
        let mut seen_plugins: HashSet<String> = HashSet::new();

        for asset in assets {
            // Register this plugin before filtering — protects it from pruning
            // even if the batch only contains non-manifest files for it.
            let safe_name = sanitize_name(&asset.plugin_name);
            seen_plugins.insert(safe_name.clone());

            // Only process plugin.json manifest files
            let rel_str = asset.relative_path.to_string_lossy();
            if !rel_str.ends_with("plugin.json") || !rel_str.contains(".claude-plugin") {
                continue;
            }

            let manifest_dir = local_dir.join(&safe_name).join(".cursor-plugin");
            let manifest_path = manifest_dir.join("plugin.json");

            // Check if unchanged
            if manifest_path.exists() {
                let existing = match fs::read(&manifest_path) {
                    Ok(data) => data,
                    Err(e) => {
                        debug!(
                            path = %manifest_path.display(),
                            error = %e,
                            "Could not read existing manifest for hash comparison, will re-write"
                        );
                        vec![]
                    }
                };
                if crate::adapters::utils::hash_content(&existing) == asset.hash {
                    report.skipped.push(SkipReason::Unchanged {
                        item: format!("{}/.cursor-plugin/plugin.json", safe_name),
                    });
                    continue;
                }
            }

            fs::create_dir_all(&manifest_dir)?;
            fs::write(&manifest_path, &asset.content)?;

            debug!(
                plugin = %safe_name,
                "Wrote plugin manifest to Cursor plugins/local"
            );
            report.written += 1;
        }

        // Prune plugins that are no longer installed.
        //
        // NI15 (PR #218 review): per-entry I/O errors used to be silently
        // dropped via `.filter_map(|e| e.ok())`, which produced partial sync
        // with no indication. Surface them as warnings on the report so the
        // operator sees that pruning was incomplete; the directory walk
        // continues with the remaining entries (vs. bailing) because a
        // single unreadable entry should not block sync of unrelated
        // plugins. Bubbling the read_dir handle itself is unchanged.
        match fs::read_dir(&local_dir) {
            Ok(entries) => {
                for entry_result in entries {
                    let entry = match entry_result {
                        Ok(e) => e,
                        Err(e) => {
                            let msg = format!(
                                "Skipped a plugins/local entry due to I/O error: {} (pruning may be incomplete)",
                                e
                            );
                            tracing::warn!(error = %e, "Failed to read plugins/local entry");
                            report.warnings.push(msg);
                            continue;
                        }
                    };
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let name = match entry.file_name().into_string() {
                        Ok(n) => n,
                        Err(raw) => {
                            let msg = format!(
                                "Skipped non-UTF-8 plugins/local directory name: {:?}",
                                raw
                            );
                            tracing::warn!(?raw, "Skipping non-UTF-8 plugin directory");
                            report.warnings.push(msg);
                            continue;
                        }
                    };
                    if !seen_plugins.contains(&name) {
                        if let Err(e) = fs::remove_dir_all(&path) {
                            tracing::warn!(
                                plugin = %name,
                                error = %e,
                                "Failed to prune stale local plugin"
                            );
                            report
                                .warnings
                                .push(format!("Failed to prune stale plugin {}: {}", name, e));
                        } else {
                            report
                                .warnings
                                .push(format!("Pruned stale plugin: {}", name));
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    path = %local_dir.display(),
                    error = %e,
                    "Could not read plugins/local for pruning"
                );
                report.warnings.push(format!(
                    "Could not read {} for pruning: {}",
                    local_dir.display(),
                    e
                ));
            }
        }

        Ok(report)
    }
}
