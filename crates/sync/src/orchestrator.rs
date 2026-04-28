//! Sync orchestrator that coordinates adapters and manages sync flow.

use crate::adapters::AgentAdapter;
use crate::models::transform_model;
use crate::report::{SkipReason, SyncReport, WriteReport};
use crate::Result;
use anyhow::bail;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
    /// Skip confirmation prompts and override `skip_existing_*` flags.
    ///
    /// When `force` is true, `skip_existing_commands` and `skip_existing_instructions`
    /// are ignored — all items are written regardless.
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
    /// Sync agents (subagents)
    #[serde(default = "default_true")]
    pub sync_agents: bool,
    /// Sync hooks (lifecycle events)
    #[serde(default = "default_true")]
    pub sync_hooks: bool,
    /// Sync instructions (CLAUDE.md → *.instructions.md)
    #[serde(default = "default_true")]
    pub sync_instructions: bool,
    /// Skip overwriting existing instructions on the target (only add new ones)
    #[serde(default)]
    pub skip_existing_instructions: bool,
    /// Include marketplace content (e.g. uninstalled plugins)
    #[serde(default)]
    pub include_marketplace: bool,
    /// Sync plugin assets (scripts, binaries, libraries that skills/hooks depend on)
    #[serde(default = "default_true")]
    pub sync_plugin_assets: bool,
    /// Interactive mode: preview diffs and accept/reject each file before writing
    #[serde(default)]
    pub interactive: bool,
    /// Exclude skills and assets from these plugins (e.g., ["phantom", "scry"]).
    /// Skills without a plugin origin are never excluded.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_plugins: Vec<String>,
    /// When true, plugin_assets includes the full plugin directory
    /// (skills, manifests) instead of only supplementary files.
    /// Used when the target needs a complete plugin mirror (e.g., Cursor).
    #[serde(default)]
    pub full_plugin_mirror: bool,
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
            sync_agents: true,
            sync_hooks: true,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
            sync_plugin_assets: true,
            interactive: false,
            exclude_plugins: Vec::new(),
            full_plugin_mirror: false,
        }
    }
}

impl SyncParams {
    /// Returns true if the given plugin name should be excluded from sync.
    pub fn is_plugin_excluded(&self, plugin_name: &str) -> bool {
        self.exclude_plugins
            .iter()
            .any(|excluded| excluded.eq_ignore_ascii_case(plugin_name))
    }
}

fn default_true() -> bool {
    true
}

/// Applies force/dry_run/skip_existing policy when syncing a collection of named items.
///
/// Encapsulates the shared conditional logic used by both commands and instructions
/// sync paths to avoid duplication.
fn sync_items<Item>(
    items: Vec<Item>,
    force: bool,
    dry_run: bool,
    skip_existing: bool,
    get_name: impl Fn(&Item) -> String,
    read_existing: impl FnOnce() -> Result<Vec<Item>>,
    write_items: impl FnOnce(&[Item]) -> Result<WriteReport>,
) -> Result<WriteReport> {
    if force || !skip_existing {
        if dry_run {
            return Ok(WriteReport {
                written: items.len(),
                ..Default::default()
            });
        }
        return write_items(&items);
    }

    // skip_existing is true (and not forced): partition into new vs existing
    let existing: HashSet<String> = read_existing()?
        .into_iter()
        .map(|item| get_name(&item))
        .collect();

    if dry_run {
        let mut report = WriteReport::default();
        for item in &items {
            if existing.contains(&get_name(item)) {
                report.skipped.push(SkipReason::WouldOverwrite {
                    item: get_name(item),
                });
            } else {
                report.written += 1;
            }
        }
        return Ok(report);
    }

    let mut new_items = Vec::new();
    let mut skipped = Vec::new();

    for item in items {
        let name = get_name(&item);
        if existing.contains(&name) {
            skipped.push(SkipReason::WouldOverwrite { item: name });
        } else {
            new_items.push(item);
        }
    }

    let mut report = if new_items.is_empty() {
        WriteReport::default()
    } else {
        write_items(&new_items)?
    };

    report.skipped.extend(skipped);
    Ok(report)
}

/// Detects duplicate names in a collection, warns about each collision, and deduplicates
/// by keeping the first occurrence (highest-priority source).
///
/// Returns the deduplicated list and the count of dropped duplicates.
fn dedup_by_name<Item>(
    items: Vec<Item>,
    kind: &str,
    get_name: impl Fn(&Item) -> String,
    get_source: impl Fn(&Item) -> String,
) -> (Vec<Item>, usize) {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut deduped = Vec::with_capacity(items.len());
    let mut dup_count = 0;

    for item in items {
        let name = get_name(&item);
        let source = get_source(&item);
        if let Some(first_source) = seen.get(&name) {
            tracing::warn!(
                kind = kind,
                name = %name,
                kept = %first_source,
                dropped = %source,
                "Duplicate {kind} /{name}: keeping from {first_source}, dropping from {source}",
            );
            dup_count += 1;
        } else {
            seen.insert(name, source);
            deduped.push(item);
        }
    }

    (deduped, dup_count)
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

    /// Performs the sync operation.
    ///
    /// Logs [`crate::adapters::traits::FieldSupport`] mismatches for observability but always delegates
    /// to the target adapter — adapters may implement creative mappings for
    /// fields they don't "natively" support (e.g., Copilot maps commands to
    /// prompts, Codex converts agents to skills).
    pub fn sync(&self, params: &SyncParams) -> Result<SyncReport> {
        let mut report = SyncReport::new();
        let target_support = self.target.supported_fields();

        // Sync commands
        if params.sync_commands {
            if !target_support.commands {
                tracing::debug!(
                    target = %self.target.name(),
                    "Target does not natively support commands; delegating to adapter"
                );
            }
            let commands = self.source.read_commands(params.include_marketplace)?;
            // Apply plugin exclusion filter to commands
            let commands: Vec<_> = if params.exclude_plugins.is_empty() {
                commands
            } else {
                commands
                    .into_iter()
                    .filter(|c| {
                        c.plugin_origin
                            .as_ref()
                            .is_none_or(|o| !params.is_plugin_excluded(&o.plugin_name))
                    })
                    .collect()
            };
            let (commands, cmd_dups) = dedup_by_name(
                commands,
                "command",
                |c| c.name.clone(),
                |c| c.source_path.display().to_string(),
            );
            let include_marketplace = params.include_marketplace;
            report.commands = sync_items(
                commands,
                params.force,
                params.dry_run,
                params.skip_existing_commands,
                |c| c.name.clone(),
                || self.target.read_commands(include_marketplace),
                |items| self.target.write_commands(items),
            )?;
            report.commands.duplicates = cmd_dups;
        }

        // Sync skills
        if params.sync_skills {
            if !target_support.skills {
                tracing::debug!(
                    target = %self.target.name(),
                    "Target does not natively support skills; delegating to adapter"
                );
            }
            let skills = self.source.read_skills()?;
            // Apply plugin exclusion filter
            let (skills, excluded_count) = if params.exclude_plugins.is_empty() {
                (skills, 0usize)
            } else {
                let before = skills.len();
                let filtered: Vec<_> = skills
                    .into_iter()
                    .filter(|s| {
                        s.plugin_origin
                            .as_ref()
                            .is_none_or(|o| !params.is_plugin_excluded(&o.plugin_name))
                    })
                    .collect();
                let excluded = before - filtered.len();
                if excluded > 0 {
                    tracing::info!(excluded, "Excluded skills from filtered plugins");
                }
                (filtered, excluded)
            };
            let (skills, skill_dups) = dedup_by_name(
                skills,
                "skill",
                |s| s.name.clone(),
                |s| s.source_path.display().to_string(),
            );
            if !params.dry_run {
                report.skills = self.target.write_skills(&skills)?;
            } else {
                report.skills.written = skills.len();
            }
            report.skills.duplicates = skill_dups;
            // Report excluded plugins as skipped
            for _ in 0..excluded_count {
                report.skills.skipped.push(SkipReason::PluginExcluded);
            }
        }

        // Sync MCP servers
        if params.sync_mcp_servers {
            if !target_support.mcp_servers {
                tracing::debug!(
                    target = %self.target.name(),
                    "Target does not natively support MCP servers; delegating to adapter"
                );
            }
            let servers = self.source.read_mcp_servers()?;
            if !params.dry_run {
                report.mcp_servers = self.target.write_mcp_servers(&servers)?;
            } else {
                report.mcp_servers.written = servers.len();
            }
        }

        // Sync preferences (with model transformation)
        if params.sync_preferences {
            if !target_support.preferences {
                tracing::debug!(
                    target = %self.target.name(),
                    "Target does not natively support preferences; delegating to adapter"
                );
            }
            let mut prefs = self.source.read_preferences()?;

            // Transform model name to target platform equivalent
            if let Some(ref model) = prefs.model {
                if let Some(transformed) =
                    transform_model(model, self.source.name(), self.target.name())
                {
                    prefs.model = Some(transformed);
                } else {
                    tracing::debug!(
                        model = %model,
                        source = %self.source.name(),
                        target = %self.target.name(),
                        "Unknown model passed through without transformation"
                    );
                }
            }

            if !params.dry_run {
                report.preferences = self.target.write_preferences(&prefs)?;
            } else if prefs.model.is_some() {
                report.preferences.written += 1;
            }
        }

        // Sync agents (subagents)
        if params.sync_agents {
            if !target_support.agents {
                tracing::debug!(
                    target = %self.target.name(),
                    "Target does not natively support agents; delegating to adapter"
                );
            }
            let agents = self.source.read_agents()?;
            // Apply plugin exclusion filter to agents
            let agents: Vec<_> = if params.exclude_plugins.is_empty() {
                agents
            } else {
                agents
                    .into_iter()
                    .filter(|a| {
                        a.plugin_origin
                            .as_ref()
                            .is_none_or(|o| !params.is_plugin_excluded(&o.plugin_name))
                    })
                    .collect()
            };
            if !params.dry_run {
                report.agents = self.target.write_agents(&agents)?;
            } else {
                report.agents.written = agents.len();
            }
        }

        // Sync hooks (lifecycle events)
        if params.sync_hooks {
            if !target_support.hooks {
                tracing::debug!(
                    target = %self.target.name(),
                    "Target does not natively support hooks; delegating to adapter"
                );
            }
            let hooks = self.source.read_hooks()?;
            if !params.dry_run {
                report.hooks = self.target.write_hooks(&hooks)?;
            } else {
                report.hooks.written = hooks.len();
            }
        }

        // Sync instructions (CLAUDE.md → *.instructions.md / .cursor/rules/*.mdc)
        if params.sync_instructions {
            if !target_support.instructions {
                tracing::debug!(
                    target = %self.target.name(),
                    "Target does not natively support instructions; delegating to adapter"
                );
            }
            let instructions = self.source.read_instructions()?;
            report.instructions = sync_items(
                instructions,
                params.force,
                params.dry_run,
                params.skip_existing_instructions,
                |i| i.name.clone(),
                || self.target.read_instructions(),
                |items| self.target.write_instructions(items),
            )?;
        }

        // Sync plugin assets (scripts, binaries, libraries)
        if params.sync_plugin_assets {
            if !target_support.plugin_assets {
                tracing::debug!(
                    target = %self.target.name(),
                    "Target does not natively support plugin assets; skipping"
                );
            } else {
                let assets = self.source.read_plugin_assets(params.full_plugin_mirror)?;
                // Apply plugin exclusion filter to assets
                let (assets, excluded_asset_count) = if params.exclude_plugins.is_empty() {
                    (assets, 0usize)
                } else {
                    let before = assets.len();
                    let filtered: Vec<_> = assets
                        .into_iter()
                        .filter(|a| !params.is_plugin_excluded(&a.plugin_name))
                        .collect();
                    let excluded = before - filtered.len();
                    (filtered, excluded)
                };
                if !params.dry_run {
                    report.plugin_assets = self.target.write_plugin_assets(&assets)?;
                } else {
                    report.plugin_assets.written = assets.len();
                }
                for _ in 0..excluded_asset_count {
                    report
                        .plugin_assets
                        .skipped
                        .push(SkipReason::PluginExcluded);
                }
            }
        }

        report.success = true;
        report.summary = report.format_summary(self.source.name(), self.target.name());

        Ok(report)
    }
}

/// Runs a sync between two named platforms using `create_adapter`.
///
/// This avoids the combinatorial match arm explosion that occurs when each
/// (from, to) pair is constructed explicitly.
pub fn sync_between(from: &str, to: &str, params: &SyncParams) -> Result<SyncReport> {
    let source = create_adapter(from)?;
    let target = create_adapter(to)?;
    SyncOrchestrator::new(source, target).sync(params)
}

/// Validates that a platform name is recognized.
///
/// ```
/// use skrills_sync::orchestrator::is_valid_platform;
///
/// assert!(is_valid_platform("claude"));
/// assert!(is_valid_platform("cursor"));
/// assert!(!is_valid_platform("vscode"));
/// ```
pub fn is_valid_platform(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "claude" | "codex" | "copilot" | "cursor"
    )
}

/// Creates an adapter for the given platform name.
///
/// Returns a boxed `AgentAdapter` for the specified platform.
pub fn create_adapter(platform: &str) -> Result<Box<dyn AgentAdapter>> {
    match platform.to_lowercase().as_str() {
        "claude" => Ok(Box::new(crate::adapters::ClaudeAdapter::new()?)),
        "codex" => Ok(Box::new(crate::adapters::CodexAdapter::new()?)),
        "copilot" => Ok(Box::new(crate::adapters::CopilotAdapter::new()?)),
        "cursor" => Ok(Box::new(crate::adapters::CursorAdapter::new()?)),
        _ => bail!(
            "Unknown platform '{}'. Use 'claude', 'codex', 'copilot', or 'cursor'",
            platform
        ),
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
    fn sync_transforms_model_claude_to_codex() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Create source settings with Claude model
        let settings_path = src_dir.path().join("settings.json");
        fs::write(&settings_path, r#"{"model": "sonnet"}"#).unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: true,
            sync_skills: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.preferences.written, 1);

        // Verify model was transformed to OpenAI equivalent
        let tgt_config = tgt_dir.path().join("config.json");
        let content = fs::read_to_string(&tgt_config).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(settings["model"], "gpt-4o-mini");
    }

    #[test]
    fn sync_transforms_model_codex_to_claude() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Create source config with OpenAI model
        let config_path = src_dir.path().join("config.json");
        fs::write(&config_path, r#"{"model": "gpt-4o"}"#).unwrap();

        let source = CodexAdapter::with_root(src_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: true,
            sync_skills: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.preferences.written, 1);

        // Verify model was transformed to Claude equivalent
        let tgt_settings = tgt_dir.path().join("settings.json");
        let content = fs::read_to_string(&tgt_settings).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(settings["model"], "opus");
    }

    #[test]
    fn sync_preserves_unknown_model() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Create source settings with unknown model
        let settings_path = src_dir.path().join("settings.json");
        fs::write(&settings_path, r#"{"model": "custom-model-v1"}"#).unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: true,
            sync_skills: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.preferences.written, 1);

        // Unknown model should be passed through unchanged
        let tgt_config = tgt_dir.path().join("config.json");
        let content = fs::read_to_string(&tgt_config).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(settings["model"], "custom-model-v1");
    }

    #[test]
    fn skip_existing_instructions_does_not_overwrite() {
        use crate::adapters::CopilotAdapter;

        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Source (Claude) has a CLAUDE.md file - this becomes an instruction
        let src_claude_md = src_dir.path().join("CLAUDE.md");
        fs::write(&src_claude_md, "# New Instructions").unwrap();

        // Target (Copilot) already has instructions
        let tgt_instr_dir = tgt_dir.path().join("instructions");
        fs::create_dir_all(&tgt_instr_dir).unwrap();
        fs::write(
            tgt_instr_dir.join("CLAUDE.instructions.md"),
            "# Existing Instructions",
        )
        .unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            sync_instructions: true,
            skip_existing_instructions: true,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.instructions.written, 0);
        assert_eq!(report.instructions.skipped.len(), 1);

        // Ensure target file was not overwritten
        let tgt_file = tgt_dir.path().join("instructions/CLAUDE.instructions.md");
        assert_eq!(
            fs::read_to_string(&tgt_file).unwrap(),
            "# Existing Instructions"
        );
    }

    #[test]
    fn skip_existing_instructions_still_writes_new_items() {
        use crate::adapters::CopilotAdapter;

        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Source (Claude) has CLAUDE.md
        let src_claude_md = src_dir.path().join("CLAUDE.md");
        fs::write(&src_claude_md, "# New Instructions").unwrap();

        // Target (Copilot) has different instruction (not CLAUDE)
        let tgt_instr_dir = tgt_dir.path().join("instructions");
        fs::create_dir_all(&tgt_instr_dir).unwrap();
        fs::write(
            tgt_instr_dir.join("other.instructions.md"),
            "# Other Instructions",
        )
        .unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            sync_instructions: true,
            skip_existing_instructions: true,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        // New instruction should be written
        assert_eq!(report.instructions.written, 1);
        assert_eq!(report.instructions.skipped.len(), 0);

        // New instruction should exist
        let new_file = tgt_dir.path().join("instructions/CLAUDE.instructions.md");
        assert!(new_file.exists());
    }

    #[test]
    fn sync_with_empty_source_commands() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Source has no commands directory at all
        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            sync_agents: false,
            sync_instructions: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.commands.written, 0);
        assert!(report.success);
    }

    #[test]
    fn force_and_dry_run_combination() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        let src_cmd_dir = src_dir.path().join("commands");
        fs::create_dir_all(&src_cmd_dir).unwrap();
        fs::write(src_cmd_dir.join("cmd.md"), "# Command").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            force: true,
            dry_run: true,
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            sync_agents: false,
            sync_instructions: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        // dry_run + force: should report written but not actually write
        assert_eq!(report.commands.written, 1);
        let tgt_file = tgt_dir.path().join("prompts/cmd.md");
        assert!(
            !tgt_file.exists(),
            "dry_run should not create files even with force"
        );
    }

    #[test]
    fn skip_existing_with_all_items_already_existing() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        let src_cmd_dir = src_dir.path().join("commands");
        fs::create_dir_all(&src_cmd_dir).unwrap();
        fs::write(src_cmd_dir.join("a.md"), "# A new").unwrap();
        fs::write(src_cmd_dir.join("b.md"), "# B new").unwrap();

        let tgt_cmd_dir = tgt_dir.path().join("prompts");
        fs::create_dir_all(&tgt_cmd_dir).unwrap();
        fs::write(tgt_cmd_dir.join("a.md"), "# A existing").unwrap();
        fs::write(tgt_cmd_dir.join("b.md"), "# B existing").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            sync_agents: false,
            sync_instructions: false,
            skip_existing_commands: true,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.commands.written, 0);
        assert_eq!(report.commands.skipped.len(), 2);
        // Existing content preserved
        assert_eq!(
            fs::read_to_string(tgt_cmd_dir.join("a.md")).unwrap(),
            "# A existing"
        );
        assert_eq!(
            fs::read_to_string(tgt_cmd_dir.join("b.md")).unwrap(),
            "# B existing"
        );
    }

    #[test]
    fn sync_nothing_enabled() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            sync_agents: false,
            sync_instructions: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert!(report.success);
        assert_eq!(report.commands.written, 0);
        assert_eq!(report.skills.written, 0);
    }

    #[test]
    fn dry_run_skip_existing_reports_skipped() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        let src_cmd_dir = src_dir.path().join("commands");
        fs::create_dir_all(&src_cmd_dir).unwrap();
        fs::write(src_cmd_dir.join("existing.md"), "# New").unwrap();

        let tgt_cmd_dir = tgt_dir.path().join("prompts");
        fs::create_dir_all(&tgt_cmd_dir).unwrap();
        fs::write(tgt_cmd_dir.join("existing.md"), "# Old").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let orchestrator = SyncOrchestrator::new(source, target);
        let params = SyncParams {
            dry_run: true,
            skip_existing_commands: true,
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            sync_agents: false,
            sync_instructions: false,
            ..Default::default()
        };

        let report = orchestrator.sync(&params).unwrap();
        assert_eq!(report.commands.written, 0);
        assert_eq!(report.commands.skipped.len(), 1);
    }

    #[test]
    fn create_adapter_all_platforms() {
        // These only verify the adapter constructs; the actual root may not exist
        // on the test machine, but with_root tests cover that path.
        assert_eq!(create_adapter("claude").unwrap().name(), "claude");
        assert_eq!(create_adapter("codex").unwrap().name(), "codex");
        assert_eq!(create_adapter("copilot").unwrap().name(), "copilot");
        assert_eq!(create_adapter("cursor").unwrap().name(), "cursor");
        assert!(create_adapter("vscode").is_err());
    }
}
