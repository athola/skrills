//! Sync orchestrator that coordinates adapters and manages sync flow.

use crate::adapters::AgentAdapter;
use crate::common::Command;
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
    /// are ignored, all items are written regardless.
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
    /// Reserved for a per-file preview-and-confirm mode that is not
    /// implemented. [`SyncOrchestrator::sync`] refuses to run when it is set,
    /// rather than writing without the confirmation it promises.
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
fn sync_items(
    items: Vec<Command>,
    force: bool,
    dry_run: bool,
    skip_existing: bool,
    read_existing: impl FnOnce() -> Result<Vec<Command>>,
    write_items: impl FnOnce(&[Command]) -> Result<WriteReport>,
) -> Result<WriteReport> {
    let get_name = |c: &Command| c.name.clone();
    if force || !skip_existing {
        if dry_run {
            return Ok(preview_items(&items, read_existing));
        }
        return write_items(&items);
    }

    // skip_existing is true (and not forced): partition into new vs existing
    let existing: HashSet<String> = read_existing()?
        .into_iter()
        .map(|item| existing_key(&get_name(&item)))
        .collect();

    if dry_run {
        let mut report = WriteReport::default();
        for item in &items {
            if existing.contains(&existing_key(&get_name(item))) {
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
        if existing.contains(&existing_key(&name)) {
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

/// Dry-run count for file artifacts, made the way the writers decide.
///
/// An item whose target copy (matched by [`existing_key`]) already holds the
/// same bytes and every source module unchanged is `Unchanged`; anything else
/// counts as written. Exact for targets that copy bytes (Claude, Codex);
/// a target that transforms content on write (Cursor, Copilot agents) never
/// matches, so its preview can still over-count, never under-count. A target
/// that cannot be read previews every item as written.
fn preview_items(
    items: &[Command],
    read_target: impl FnOnce() -> Result<Vec<Command>>,
) -> WriteReport {
    let target = read_target().unwrap_or_else(|e| {
        tracing::debug!(error = %format!("{e:#}"), "Dry run could not read the target; counting every item");
        Vec::new()
    });
    let by_key: HashMap<String, &Command> =
        target.iter().map(|c| (existing_key(&c.name), c)).collect();
    let mut report = WriteReport::default();
    for item in items {
        let unchanged = by_key.get(&existing_key(&item.name)).is_some_and(|t| {
            t.content == item.content
                && item.modules.iter().all(|m| {
                    t.modules
                        .iter()
                        .any(|tm| tm.relative_path == m.relative_path && tm.content == m.content)
                })
        });
        if unchanged {
            report.skipped.push(SkipReason::Unchanged {
                item: item.name.clone(),
            });
        } else {
            report.written += 1;
        }
    }
    report
}

/// Dry-run count for MCP servers: a server the target already holds with the
/// same settings is `Unchanged`.
fn preview_servers(
    servers: &HashMap<String, crate::common::McpServer>,
    target: &HashMap<String, crate::common::McpServer>,
) -> WriteReport {
    let mut report = WriteReport::default();
    for (name, server) in servers {
        if target.get(name) == Some(server) {
            report
                .skipped
                .push(SkipReason::Unchanged { item: name.clone() });
        } else {
            report.written += 1;
        }
    }
    report
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

/// Records an `UnsupportedField` skip when the target declares it cannot
/// write `field`, and says whether the phase may run.
///
/// The flags used to be advisory: the orchestrator logged and called the
/// writer anyway, so a target whose writer is a no-op reported success with
/// nothing written and no reason given.
fn target_writes(
    supported: bool,
    field: &str,
    source: &str,
    target: &str,
    slot: &mut WriteReport,
) -> bool {
    if !supported {
        tracing::debug!(target, field, "Target cannot write this artifact; skipping");
        slot.skipped.push(SkipReason::UnsupportedField {
            field: field.to_string(),
            source_agent: source.to_string(),
            suggestion: format!("{target} has no place for {field}"),
        });
    }
    supported
}

/// Runs one sync phase into its slot of the report.
///
/// A failure is written into the slot as a warning and marks the run failed,
/// and the remaining phases still run. The kill switch is the exception: once
/// it is engaged no phase may write, so its error ends the sync.
fn run_phase(
    name: &str,
    slot: &mut WriteReport,
    failed: &mut bool,
    phase: impl FnOnce(&mut WriteReport) -> Result<()>,
) -> Result<()> {
    let Err(e) = phase(slot) else {
        return Ok(());
    };
    if matches!(
        e.downcast_ref::<crate::error::SyncError>(),
        Some(crate::error::SyncError::TokenBudgetExceeded { .. })
    ) {
        return Err(e);
    }
    tracing::error!(phase = name, error = %format!("{e:#}"), "Sync phase failed");
    slot.warnings.push(format!("{name} sync failed: {e:#}"));
    *failed = true;
    Ok(())
}

/// Drops items that came from a plugin named in `exclude_plugins`. Items with
/// no plugin origin are always kept.
fn without_excluded_plugins(
    items: Vec<crate::common::Command>,
    params: &SyncParams,
) -> Vec<crate::common::Command> {
    if params.exclude_plugins.is_empty() {
        return items;
    }
    items
        .into_iter()
        .filter(|item| {
            item.plugin_origin
                .as_ref()
                .is_none_or(|o| !params.is_plugin_excluded(&o.plugin_name))
        })
        .collect()
}

/// Whether two tools take model ids from the same vendor, so an id with no
/// mapping can still be carried over unchanged.
fn same_model_vendor(source: &str, target: &str) -> bool {
    let openai = |name: &str| matches!(name, "codex" | "copilot");
    source == target || (openai(source) && openai(target))
}

/// The form of a name that survives every adapter's sanitizer: writers strip
/// `.`, spaces and `/` or kebab-case, so `my.cmd`, `my-cmd` and `mycmd` can all
/// land in one file. Comparing these keys errs towards "already exists", which
/// in skip-existing mode means keeping the target copy.
fn existing_key(name: &str) -> String {
    let key: String = name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if key.is_empty() {
        name.to_string()
    } else {
        key
    }
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
    /// to the target adapter, adapters may implement creative mappings for
    /// fields they don't "natively" support (e.g., Copilot maps commands to
    /// prompts, Codex converts agents to skills).
    pub fn sync(&self, params: &SyncParams) -> Result<SyncReport> {
        if params.interactive {
            // Nothing implements the per-file preview and confirmation this
            // flag promises; running anyway would write without asking.
            bail!("interactive sync is not implemented; preview with dry_run instead");
        }

        let mut report = SyncReport::new();
        let target_support = self.target.write_support();
        let source_support = self.source.read_support();
        let (source_name, target_name) = (self.source.name(), self.target.name());

        // A full plugin mirror already carries every plugin skill body into the
        // target's plugin tree, so the flat skill copy would be a second copy of
        // the same file. This is the single place that can decide it: callers
        // see neither the support flags nor which skills came from a plugin.
        let mirror_carries_plugin_skills = params.sync_plugin_assets
            && params.full_plugin_mirror
            && source_support.plugin_assets
            && target_support.plugin_assets;

        // Each phase runs on its own: one that fails is recorded in its slot of
        // the report and the rest still run, so the caller learns what was
        // already written instead of getting a bare error.
        let mut failed = false;

        // Sync commands
        if params.sync_commands
            && target_writes(
                target_support.commands,
                "commands",
                source_name,
                target_name,
                &mut report.commands,
            )
        {
            run_phase("commands", &mut report.commands, &mut failed, |slot| {
                let commands = self.source.read_commands(params.include_marketplace)?;
                let commands = without_excluded_plugins(commands, params);
                let (commands, cmd_dups) = dedup_by_name(
                    commands,
                    "command",
                    |c| c.name.clone(),
                    |c| c.source_path.display().to_string(),
                );
                let include_marketplace = params.include_marketplace;
                *slot = sync_items(
                    commands,
                    params.force,
                    params.dry_run,
                    params.skip_existing_commands,
                    || self.target.read_commands(include_marketplace),
                    |items| self.target.write_commands(items),
                )?;
                slot.duplicates = cmd_dups;
                Ok(())
            })?;
        }

        // Sync skills
        if params.sync_skills
            && target_writes(
                target_support.skills,
                "skills",
                source_name,
                target_name,
                &mut report.skills,
            )
        {
            run_phase("skills", &mut report.skills, &mut failed, |slot| {
                let mut skills = self.source.read_skills()?;
                if mirror_carries_plugin_skills {
                    let before = skills.len();
                    skills.retain(|s| s.plugin_origin.is_none());
                    let in_mirror = before - skills.len();
                    if in_mirror > 0 {
                        tracing::debug!(
                            in_mirror,
                            "Plugin skills ride the plugin mirror; not copying them flat as well"
                        );
                    }
                }
                let before = skills.len();
                let skills = without_excluded_plugins(skills, params);
                let excluded_count = before - skills.len();
                if excluded_count > 0 {
                    tracing::info!(
                        excluded = excluded_count,
                        "Excluded skills from filtered plugins"
                    );
                }
                let (skills, skill_dups) = dedup_by_name(
                    skills,
                    "skill",
                    |s| s.name.clone(),
                    |s| s.source_path.display().to_string(),
                );
                if !params.dry_run {
                    *slot = self.target.write_skills(&skills)?;
                } else {
                    *slot = preview_items(&skills, || self.target.read_skills());
                }
                slot.duplicates = skill_dups;
                // Report excluded plugins as skipped
                for _ in 0..excluded_count {
                    slot.skipped.push(SkipReason::PluginExcluded);
                }
                Ok(())
            })?;
        }

        // Sync MCP servers
        if params.sync_mcp_servers
            && target_writes(
                target_support.mcp_servers,
                "mcp servers",
                source_name,
                target_name,
                &mut report.mcp_servers,
            )
        {
            run_phase(
                "MCP servers",
                &mut report.mcp_servers,
                &mut failed,
                |slot| {
                    let servers = self.source.read_mcp_servers()?;
                    if !params.dry_run {
                        *slot = self.target.write_mcp_servers(&servers)?;
                    } else {
                        let target = self.target.read_mcp_servers().unwrap_or_default();
                        *slot = preview_servers(&servers, &target);
                    }
                    Ok(())
                },
            )?;
        }

        // Sync preferences (with model transformation)
        if params.sync_preferences
            && target_writes(
                target_support.preferences,
                "preferences",
                source_name,
                target_name,
                &mut report.preferences,
            )
        {
            run_phase(
                "preferences",
                &mut report.preferences,
                &mut failed,
                |slot| {
                    let mut prefs = self.source.read_preferences()?;
                    let mut untranslated = None;

                    // Transform model name to target platform equivalent
                    if let Some(model) = prefs.model.take() {
                        let (source, target) = (self.source.name(), self.target.name());
                        match transform_model(&model, source, target) {
                            Some(transformed) => prefs.model = Some(transformed),
                            None if same_model_vendor(source, target) => prefs.model = Some(model),
                            None => {
                                // Another vendor's tool rejects an id it does not
                                // know, so the target keeps the model it has.
                                tracing::debug!(
                                    model = %model,
                                    source = %source,
                                    target = %target,
                                    "Model has no equivalent on target; leaving target model alone"
                                );
                                untranslated = Some(format!(
                                "Model `{model}` from {source} has no known {target} equivalent; \
                                 the {target} model was left unchanged"
                            ));
                            }
                        }
                    }

                    if !params.dry_run {
                        *slot = self.target.write_preferences(&prefs)?;
                    } else if let Some(model) = &prefs.model {
                        let current = self.target.read_preferences().ok().and_then(|p| p.model);
                        if current.as_ref() == Some(model) {
                            slot.skipped.push(SkipReason::Unchanged {
                                item: "model".to_string(),
                            });
                        } else {
                            slot.written += 1;
                        }
                    }
                    slot.warnings.extend(untranslated);
                    Ok(())
                },
            )?;
        }

        // Sync agents (subagents)
        if params.sync_agents
            && target_writes(
                target_support.agents,
                "agents",
                source_name,
                target_name,
                &mut report.agents,
            )
        {
            run_phase("agents", &mut report.agents, &mut failed, |slot| {
                let agents = without_excluded_plugins(self.source.read_agents()?, params);
                if !params.dry_run {
                    *slot = self.target.write_agents(&agents)?;
                } else {
                    *slot = preview_items(&agents, || self.target.read_agents());
                }
                Ok(())
            })?;
        }

        // Sync hooks (lifecycle events)
        if params.sync_hooks
            && target_writes(
                target_support.hooks,
                "hooks",
                source_name,
                target_name,
                &mut report.hooks,
            )
        {
            run_phase("hooks", &mut report.hooks, &mut failed, |slot| {
                let hooks = without_excluded_plugins(self.source.read_hooks()?, params);
                if !params.dry_run {
                    *slot = self.target.write_hooks(&hooks)?;
                } else {
                    *slot = preview_items(&hooks, || self.target.read_hooks());
                }
                Ok(())
            })?;
        }

        // Sync instructions (CLAUDE.md → *.instructions.md / .cursor/rules/*.mdc)
        if params.sync_instructions
            && target_writes(
                target_support.instructions,
                "instructions",
                source_name,
                target_name,
                &mut report.instructions,
            )
        {
            run_phase(
                "instructions",
                &mut report.instructions,
                &mut failed,
                |slot| {
                    let instructions = self.source.read_instructions()?;
                    *slot = sync_items(
                        instructions,
                        params.force,
                        params.dry_run,
                        params.skip_existing_instructions,
                        || self.target.read_instructions(),
                        |items| self.target.write_instructions(items),
                    )?;
                    Ok(())
                },
            )?;
        }

        // Sync plugin assets (scripts, binaries, libraries)
        if params.sync_plugin_assets {
            // Both ends are checked. Reading from a source with no reader
            // yields an empty Vec and writing to a target with no writer
            // reports a successful write of nothing, so an entirely
            // unimplemented pair used to come back as success.
            // Which end is missing decides which reason is the true one, so it is
            // computed once instead of building the same value twice and letting
            // the free-text suggestion carry the difference.
            let missing_end = match (source_support.plugin_assets, target_support.plugin_assets) {
                (false, _) => {
                    tracing::debug!(
                        source = %self.source.name(),
                        "Source cannot read plugin assets; skipping"
                    );
                    Some(SkipReason::SourceCannotRead {
                        field: "plugin_assets".to_string(),
                        source_agent: self.source.name().to_string(),
                    })
                }
                (true, false) => {
                    tracing::debug!(
                        target = %self.target.name(),
                        "Target does not natively support plugin assets; skipping"
                    );
                    Some(SkipReason::UnsupportedField {
                        field: "plugin_assets".to_string(),
                        source_agent: self.source.name().to_string(),
                        suggestion: format!("{} cannot write plugin assets", self.target.name()),
                    })
                }
                (true, true) => None,
            };

            if let Some(reason) = missing_end {
                report.plugin_assets.skipped.push(reason);
            } else {
                run_phase(
                    "plugin assets",
                    &mut report.plugin_assets,
                    &mut failed,
                    |slot| {
                        let assets = self.source.read_plugin_assets(params.full_plugin_mirror)?;
                        // Apply plugin exclusion filter to assets
                        let before = assets.len();
                        let assets: Vec<_> = assets
                            .into_iter()
                            .filter(|a| !params.is_plugin_excluded(&a.plugin_name))
                            .collect();
                        let excluded_asset_count = before - assets.len();
                        if !params.dry_run {
                            *slot = self.target.write_plugin_assets(&assets)?;
                        } else {
                            // The writer also deletes, so the preview has to come from
                            // the adapter rather than from the batch length alone.
                            *slot = self.target.preview_plugin_assets(&assets)?;
                        }
                        for _ in 0..excluded_asset_count {
                            slot.skipped.push(SkipReason::PluginExcluded);
                        }
                        Ok(())
                    },
                )?;
            }
        }

        report.success = !failed;
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

    /// A model id with no mapping was written verbatim into the other
    /// vendor's config, where the tool rejects it. The target keeps its own.
    #[test]
    fn sync_leaves_target_model_alone_when_source_model_has_no_equivalent() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        fs::write(
            src_dir.path().join("settings.json"),
            r#"{"model": "custom-model-v1"}"#,
        )
        .unwrap();
        fs::write(tgt_dir.path().join("config.json"), r#"{"model": "gpt-5"}"#).unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let report = SyncOrchestrator::new(source, target)
            .sync(&SyncParams {
                sync_commands: false,
                sync_mcp_servers: false,
                sync_preferences: true,
                sync_skills: false,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.preferences.written, 0);
        assert!(
            report
                .preferences
                .warnings
                .iter()
                .any(|w| w.contains("custom-model-v1")),
            "{:?}",
            report.preferences.warnings
        );
        let content = fs::read_to_string(tgt_dir.path().join("config.json")).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(settings["model"], "gpt-5");
    }

    /// Codex and Copilot share model ids, so an unmapped one still carries.
    #[test]
    fn sync_passes_an_unmapped_model_between_tools_of_one_vendor() {
        use crate::adapters::CopilotAdapter;

        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::write(src_dir.path().join("config.json"), r#"{"model": "gpt-5"}"#).unwrap();

        let source = CodexAdapter::with_root(src_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(tgt_dir.path().to_path_buf());
        let report = SyncOrchestrator::new(source, target)
            .sync(&SyncParams {
                sync_commands: false,
                sync_mcp_servers: false,
                sync_preferences: true,
                sync_skills: false,
                sync_agents: false,
                sync_hooks: false,
                sync_instructions: false,
                sync_plugin_assets: false,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.preferences.written, 1);
    }

    /// A failing phase used to discard the report of every phase that had
    /// already written files, and `success` was always true.
    #[test]
    fn a_failing_phase_keeps_the_earlier_phases_and_marks_the_report_failed() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        fs::create_dir_all(src_dir.path().join("commands")).unwrap();
        fs::write(src_dir.path().join("commands/hello.md"), "# Hello").unwrap();
        fs::write(
            src_dir.path().join("settings.json"),
            r#"{"model": "sonnet"}"#,
        )
        .unwrap();
        // A non-object root makes the preference writer fail.
        fs::write(tgt_dir.path().join("config.json"), "[]").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());
        let report = SyncOrchestrator::new(source, target)
            .sync(&SyncParams {
                sync_commands: true,
                sync_mcp_servers: false,
                sync_preferences: true,
                sync_skills: false,
                sync_agents: false,
                sync_hooks: false,
                sync_instructions: false,
                sync_plugin_assets: false,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.commands.written, 1);
        assert!(!report.success);
        assert!(
            report
                .preferences
                .warnings
                .iter()
                .any(|w| w.contains("failed")),
            "{:?}",
            report.preferences.warnings
        );
    }

    /// SY-28: dry-run reported every item as written, so an in-sync pair
    /// previewed `Skills: N synced` and then wrote nothing.
    #[test]
    fn a_dry_run_of_an_in_sync_pair_previews_no_writes() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("commands")).unwrap();
        fs::write(src_dir.path().join("commands/hello.md"), "# Hello").unwrap();
        fs::create_dir_all(src_dir.path().join("skills/alpha")).unwrap();
        fs::write(
            src_dir.path().join("skills/alpha/SKILL.md"),
            "---\nname: alpha\ndescription: A\n---\n\nBody\n",
        )
        .unwrap();
        fs::write(src_dir.path().join("skills/alpha/notes.md"), "notes").unwrap();
        fs::write(
            src_dir.path().join("settings.json"),
            r#"{"model": "sonnet", "mcpServers": {"s": {"command": "/bin/s", "args": ["-x"]}}}"#,
        )
        .unwrap();

        let params = SyncParams {
            sync_commands: true,
            sync_skills: true,
            sync_mcp_servers: true,
            sync_preferences: true,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: false,
            sync_plugin_assets: false,
            ..Default::default()
        };
        let orch = || {
            SyncOrchestrator::new(
                ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
                CodexAdapter::with_root(tgt_dir.path().to_path_buf()),
            )
        };

        let first = orch()
            .sync(&SyncParams {
                dry_run: true,
                ..params.clone()
            })
            .unwrap();
        assert_eq!(first.commands.written, 1);
        assert_eq!(first.skills.written, 1);
        assert_eq!(first.mcp_servers.written, 1);
        assert_eq!(first.preferences.written, 1);

        assert!(orch().sync(&params).unwrap().success);

        let preview = orch()
            .sync(&SyncParams {
                dry_run: true,
                ..params.clone()
            })
            .unwrap();
        let real = orch().sync(&params).unwrap();
        for (what, p, r) in [
            ("commands", &preview.commands, &real.commands),
            ("skills", &preview.skills, &real.skills),
            ("mcp", &preview.mcp_servers, &real.mcp_servers),
            ("preferences", &preview.preferences, &real.preferences),
        ] {
            assert_eq!(p.written, 0, "{what} preview: {p:?}");
            assert_eq!(r.written, 0, "{what} real: {r:?}");
        }

        // A changed module is a write in the preview too.
        fs::write(src_dir.path().join("skills/alpha/notes.md"), "new notes").unwrap();
        let preview = orch()
            .sync(&SyncParams {
                dry_run: true,
                ..params
            })
            .unwrap();
        assert_eq!(preview.skills.written, 1);
    }

    /// SY-37: Codex declares it cannot write hooks and its writer is a no-op,
    /// yet the phase ran and reported success with no reason given.
    #[test]
    fn a_phase_the_target_cannot_write_is_skipped_with_a_reason() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("hooks")).unwrap();
        fs::write(src_dir.path().join("hooks/PreToolUse.md"), "echo hi").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());
        let report = SyncOrchestrator::new(source, target)
            .sync(&SyncParams {
                sync_commands: false,
                sync_mcp_servers: false,
                sync_preferences: false,
                sync_skills: false,
                sync_agents: false,
                sync_hooks: true,
                sync_instructions: false,
                sync_plugin_assets: false,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(report.hooks.written, 0);
        assert!(
            matches!(
                report.hooks.skipped.as_slice(),
                [SkipReason::UnsupportedField { field, .. }] if field == "hooks"
            ),
            "{:?}",
            report.hooks.skipped
        );
        assert!(report.success);
    }

    /// `interactive` promised a per-file confirmation that does not exist, so
    /// a caller asking for it got an unconfirmed write.
    #[test]
    fn interactive_mode_is_refused_before_anything_is_written() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("commands")).unwrap();
        fs::write(src_dir.path().join("commands/hello.md"), "# Hello").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(tgt_dir.path().to_path_buf());
        let result = SyncOrchestrator::new(source, target).sync(&SyncParams {
            interactive: true,
            ..Default::default()
        });

        assert!(result.is_err());
        assert!(!tgt_dir.path().join("commands/hello.md").exists());
    }

    /// The writer stores `my.cmd` as `mycmd.md`; skip-existing compared the
    /// raw names, missed the match and overwrote the target's file.
    #[test]
    fn skip_existing_matches_names_the_way_the_writer_stores_them() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("commands")).unwrap();
        fs::write(src_dir.path().join("commands/my.cmd.md"), "# New").unwrap();
        fs::create_dir_all(tgt_dir.path().join("commands")).unwrap();
        fs::write(tgt_dir.path().join("commands/mycmd.md"), "# Existing").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(tgt_dir.path().to_path_buf());
        let report = SyncOrchestrator::new(source, target)
            .sync(&SyncParams {
                sync_commands: true,
                skip_existing_commands: true,
                sync_mcp_servers: false,
                sync_preferences: false,
                sync_skills: false,
                sync_agents: false,
                sync_hooks: false,
                sync_instructions: false,
                sync_plugin_assets: false,
                ..Default::default()
            })
            .unwrap();

        assert_eq!(
            fs::read_to_string(tgt_dir.path().join("commands/mycmd.md")).unwrap(),
            "# Existing"
        );
        assert_eq!(report.commands.written, 0);
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
        // dry_run and force: should report written but not actually write
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

    /// FieldSupport had no direction, so one bool meant "reads it" for one
    /// adapter and "writes it" for another. cursor->claude plugin assets
    /// passed the target gate (Claude declares plugin_assets), read nothing
    /// through Cursor's default reader, wrote nothing through Claude's default
    /// writer, and reported success.
    #[test]
    fn plugin_asset_direction_is_declared_per_adapter() {
        use crate::adapters::{ClaudeAdapter, CursorAdapter};

        let claude = ClaudeAdapter::with_root(std::path::PathBuf::from("/tmp/claude-x"));
        let cursor = CursorAdapter::with_root(std::path::PathBuf::from("/tmp/cursor-x"));

        assert!(
            claude.read_support().plugin_assets,
            "Claude owns the plugin cache and reads it"
        );
        assert!(
            !claude.write_support().plugin_assets,
            "Claude implements no plugin-asset writer"
        );
        assert!(
            cursor.write_support().plugin_assets,
            "Cursor receives the plugin mirror"
        );
        assert!(
            !cursor.read_support().plugin_assets,
            "Cursor implements no plugin-asset reader"
        );
    }

    /// The unimplemented direction must report why it did nothing rather than
    /// coming back as a clean success, and it must blame the end that is
    /// actually missing: here Cursor has no reader, while the Claude target is
    /// fine.
    #[test]
    fn cursor_to_claude_plugin_assets_reports_the_source_cannot_read() {
        use crate::adapters::{ClaudeAdapter, CursorAdapter};

        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        let orchestrator = SyncOrchestrator::new(
            CursorAdapter::with_root(src.path().to_path_buf()),
            ClaudeAdapter::with_root(dst.path().to_path_buf()),
        );

        let params = SyncParams {
            sync_plugin_assets: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            ..Default::default()
        };
        let report = orchestrator.sync(&params).unwrap();

        assert_eq!(
            report.plugin_assets.written, 0,
            "nothing is written on an unimplemented direction"
        );
        let skipped = &report.plugin_assets.skipped;
        assert!(
            skipped.iter().any(|r| matches!(
                r,
                SkipReason::SourceCannotRead { field, source_agent }
                    if field == "plugin_assets" && source_agent == "cursor"
            )),
            "the skip must name the source as the end with no reader, got {skipped:?}"
        );
        let description = skipped[0].description();
        assert!(
            description.contains("cursor") && !description.contains("target"),
            "the description must not blame the target, got {description:?}"
        );
        assert!(
            report.summary.contains(&description),
            "the summary must explain the skip, got {:?}",
            report.summary
        );
    }

    /// The CLI reaches adapters through `Box<dyn AgentAdapter>`. If the
    /// blanket impl stops forwarding `read_support`/`write_support`, the
    /// wrapper reports a direction the inner adapter does not implement and
    /// the silent no-op comes back.
    #[test]
    fn boxed_adapter_forwards_directional_support() {
        use crate::adapters::{ClaudeAdapter, CursorAdapter};

        let claude: Box<dyn AgentAdapter> = Box::new(ClaudeAdapter::with_root(
            std::path::PathBuf::from("/tmp/claude-x"),
        ));
        let cursor: Box<dyn AgentAdapter> = Box::new(CursorAdapter::with_root(
            std::path::PathBuf::from("/tmp/cursor-x"),
        ));

        assert!(claude.read_support().plugin_assets);
        assert!(
            !claude.write_support().plugin_assets,
            "Box<dyn> must forward write_support to the inner adapter"
        );
        assert!(cursor.write_support().plugin_assets);
        assert!(
            !cursor.read_support().plugin_assets,
            "Box<dyn> must forward read_support to the inner adapter"
        );
    }

    /// Seeds a Claude root with one plugin-cache skill and one skill that has
    /// no plugin origin, plus the plugin manifest the full mirror needs.
    fn seed_claude_root_with_plugin_and_local_skill(root: &std::path::Path) {
        let local = root.join("skills/local-only");
        fs::create_dir_all(&local).unwrap();
        fs::write(
            local.join("SKILL.md"),
            "---\nname: local-only\ndescription: Not from a plugin\n---\n# Local\n",
        )
        .unwrap();

        let version_dir = root.join("plugins/cache/market/my-plugin/1.0.0");
        let plugin_skill = version_dir.join("skills/deep-work");
        fs::create_dir_all(&plugin_skill).unwrap();
        fs::write(
            plugin_skill.join("SKILL.md"),
            "---\nname: deep-work\ndescription: From a plugin\n---\n# Deep work\n",
        )
        .unwrap();
        let manifest_dir = version_dir.join(".claude-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();
        fs::write(
            manifest_dir.join("plugin.json"),
            "{\"name\": \"my-plugin\", \"version\": \"1.0.0\"}\n",
        )
        .unwrap();
    }

    /// The full plugin mirror already carries plugin skill bodies into
    /// `plugins/local/<p>/skills/`, so syncing them flat as well writes every
    /// plugin skill twice. Skills with no plugin origin are not in the mirror
    /// and must still go to the flat directory.
    #[test]
    fn full_mirror_sync_skips_flat_copy_of_plugin_skills_but_keeps_the_rest() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        seed_claude_root_with_plugin_and_local_skill(src_dir.path());

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = crate::adapters::CursorAdapter::with_root(tgt_dir.path().to_path_buf());

        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: false,
            sync_skills: true,
            sync_plugin_assets: true,
            full_plugin_mirror: true,
            ..Default::default()
        };

        let report = SyncOrchestrator::new(source, target).sync(&params).unwrap();

        assert_eq!(
            report.skills.written, 1,
            "only the origin-less skill should be written flat, got {:?}",
            report.skills
        );
        assert!(
            tgt_dir.path().join("skills/local-only/SKILL.md").exists(),
            "a skill with no plugin origin still needs the flat copy"
        );
        assert!(
            !tgt_dir.path().join("skills/deep-work").exists(),
            "the plugin skill must not be duplicated into the flat directory"
        );
        assert!(
            tgt_dir
                .path()
                .join("plugins/local/my-plugin/skills/deep-work/SKILL.md")
                .exists(),
            "the plugin mirror must carry the plugin skill body"
        );
    }

    /// Without the full mirror there is no second copy to collide with, so
    /// every skill goes through `write_skills` as before.
    #[test]
    fn sync_without_full_mirror_still_writes_plugin_skills() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        seed_claude_root_with_plugin_and_local_skill(src_dir.path());

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = crate::adapters::CursorAdapter::with_root(tgt_dir.path().to_path_buf());

        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: false,
            sync_skills: true,
            sync_plugin_assets: false,
            full_plugin_mirror: false,
            ..Default::default()
        };

        let report = SyncOrchestrator::new(source, target).sync(&params).unwrap();

        assert_eq!(report.skills.written, 2, "both skills should be written");
        assert!(tgt_dir.path().join("skills/local-only/SKILL.md").exists());
        assert!(tgt_dir
            .path()
            .join("plugins/local/my-plugin/skills/deep-work/SKILL.md")
            .exists());
    }

    /// Codex declares `read_support().agents = false`, and the review asked
    /// whether that left `CodexAdapter::write_agents` unreachable from `sync`.
    /// It does not: only `plugin_assets` gates a sync, so the converted skill
    /// really does land on disk.
    #[test]
    fn claude_to_codex_sync_agents_writes_converted_skill() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        let agents_dir = src_dir.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("reviewer.md"),
            "---\nname: reviewer\ndescription: Reviews code\n---\n# Reviewer\n",
        )
        .unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());

        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            sync_hooks: false,
            sync_instructions: false,
            sync_plugin_assets: false,
            sync_agents: true,
            ..Default::default()
        };

        let report = SyncOrchestrator::new(source, target).sync(&params).unwrap();

        assert_eq!(report.agents.written, 1, "the agent should be written");
        assert!(
            tgt_dir
                .path()
                .join("skills/agent-reviewer/SKILL.md")
                .exists(),
            "write_agents should convert the agent into an agent-prefixed skill"
        );
    }

    /// A source that can read plugin assets paired with a target that cannot
    /// write them is the other half of the gate; it must name the target.
    #[test]
    fn plugin_assets_to_target_without_writer_reports_unsupported() {
        use crate::adapters::{ClaudeAdapter, CopilotAdapter};

        let src = tempdir().unwrap();
        let dst = tempdir().unwrap();
        let orchestrator = SyncOrchestrator::new(
            ClaudeAdapter::with_root(src.path().to_path_buf()),
            CopilotAdapter::with_root(dst.path().to_path_buf()),
        );

        let params = SyncParams {
            sync_plugin_assets: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            ..Default::default()
        };
        let report = orchestrator.sync(&params).unwrap();

        assert_eq!(report.plugin_assets.written, 0);
        assert!(
            report.plugin_assets.skipped.iter().any(|r| matches!(
                r,
                SkipReason::UnsupportedField { field, suggestion, .. }
                    if field == "plugin_assets" && suggestion.contains("copilot")
            )),
            "the skip must name the target that cannot write, got {:?}",
            report.plugin_assets.skipped
        );
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
