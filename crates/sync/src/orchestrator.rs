//! Sync orchestrator that coordinates adapters and manages sync flow.

use crate::adapters::AgentAdapter;
use crate::common::Command;
use crate::conflict::Ledger;
use crate::decision::{DecisionScope, ItemDecider, ItemKind};
use crate::interactive::{changed_names, file_key, DecisionBook, Gate, Resolved};
use crate::models::transform_model;
use crate::report::{SkipReason, SyncReport, WriteReport};
use crate::Result;
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
    /// Ask before writing each new or changed item and write only the ones
    /// accepted. Needs an [`ItemDecider`] on the orchestrator
    /// ([`SyncOrchestrator::with_decider`]); without one, and not a dry run,
    /// [`SyncOrchestrator::sync`] refuses rather than write unasked. A dry
    /// run writes nothing, so it previews and never asks.
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
    /// Keep target files that a newer plugin version no longer ships, and
    /// flat skill copies the plugin mirror now carries, instead of removing them.
    #[serde(default)]
    pub keep_versions: bool,
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
            keep_versions: false,
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
pub(crate) fn preview_items(
    items: &[Command],
    read_target: impl FnOnce() -> Result<Vec<Command>>,
) -> WriteReport {
    preview_items_keyed(items, read_target, |c| existing_key(&c.name))
}

/// [`preview_items`] for skills: writers put a skill in its
/// [`skill_folder_name`](crate::adapters::utils::skill_folder_name) folder,
/// so `synced/abc/morning` is read back as `morning` and has to be matched
/// under that key.
pub(crate) fn preview_skill_items(
    items: &[Command],
    read_target: impl FnOnce() -> Result<Vec<Command>>,
) -> WriteReport {
    preview_items_keyed(items, read_target, |c| {
        existing_key(&crate::adapters::utils::skill_folder_name(c))
    })
}

/// Whether the target copy `held` already carries `item`'s bytes and every
/// one of its modules.
pub(crate) fn is_unchanged(held: &Command, item: &Command) -> bool {
    held.content == item.content
        && item.modules.iter().all(|m| {
            held.modules
                .iter()
                .any(|tm| tm.relative_path == m.relative_path && tm.content == m.content)
        })
}

fn preview_items_keyed(
    items: &[Command],
    read_target: impl FnOnce() -> Result<Vec<Command>>,
    key: impl Fn(&Command) -> String,
) -> WriteReport {
    let target = read_target().unwrap_or_else(|e| {
        tracing::debug!(error = %format!("{e:#}"), "Dry run could not read the target; counting every item");
        Vec::new()
    });
    let by_key: HashMap<String, &Command> = target.iter().map(|c| (key(c), c)).collect();
    let mut report = WriteReport::default();
    for item in items {
        let unchanged = by_key
            .get(&key(item))
            .is_some_and(|t| is_unchanged(t, item));
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
        Some(
            crate::error::SyncError::TokenBudgetExceeded { .. }
                | crate::error::SyncError::KillSwitchEngaged
                | crate::error::SyncError::InteractiveAborted
        )
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

/// The form of a name that survives every adapter's sanitizer: writers strip
/// `.`, spaces and `/` or kebab-case, so `my.cmd`, `my-cmd` and `mycmd` can all
/// land in one file. Comparing these keys errs towards "already exists", which
/// in skip-existing mode means keeping the target copy.
pub(crate) fn existing_key(name: &str) -> String {
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

/// Lock file in a target's config root that serializes syncs into it.
const SYNC_LOCK_FILE: &str = ".skrills-sync.lock";

/// Waits for and takes the advisory lock on `root`'s [`SYNC_LOCK_FILE`]. The
/// lock is released when the returned file is dropped.
///
/// Opens the file directly, not through the recording helpers, and the
/// raw-mutation test allows it by name: the lock is not user content, and a
/// rollback that removed it could pull it from under a running sync.
fn lock_target(root: &std::path::Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(root)
        .with_context(|| format!("Failed to create {}", root.display()))?;
    let path = root.join(SYNC_LOCK_FILE);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| format!("Failed to open {}", path.display()))?;
    file.lock()
        .with_context(|| format!("Failed to lock {}", path.display()))?;
    Ok(file)
}

/// Orchestrates sync operations between agents.
pub struct SyncOrchestrator<S: AgentAdapter, T: AgentAdapter> {
    source: S,
    target: T,
    /// Where a `skrills cold-window` process in another process records an
    /// engaged kill switch; see [`crate::kill_switch_marker`].
    kill_switch_marker: Option<std::path::PathBuf>,
    /// Where each sync's pre-write snapshot goes; `None` takes none. Library
    /// callers start without one so embedding the orchestrator never writes
    /// outside the target; the CLI and MCP entry points opt in.
    snapshot_root: Option<std::path::PathBuf>,
    /// Answers the per-item questions of an interactive sync.
    decider: Option<Arc<dyn ItemDecider>>,
    /// Where the per-target baselines and the resolution log live; `None`
    /// detects no conflicts and records nothing. Library callers start
    /// without one, for the reason `snapshot_root` does.
    state_root: Option<std::path::PathBuf>,
}

impl<S: AgentAdapter, T: AgentAdapter> SyncOrchestrator<S, T> {
    /// Creates a new orchestrator with source and target adapters. It
    /// consults the kill-switch marker at
    /// [`crate::kill_switch_marker::marker_path`].
    pub fn new(source: S, target: T) -> Self {
        Self {
            source,
            target,
            kill_switch_marker: crate::kill_switch_marker::marker_path(),
            snapshot_root: None,
            decider: None,
            state_root: None,
        }
    }

    /// Keeps the conflict baseline and resolution log under `root`. `None`
    /// keeps none.
    #[must_use]
    pub fn with_state_root(mut self, root: Option<std::path::PathBuf>) -> Self {
        self.state_root = root;
        self
    }

    /// [`Self::with_state_root`] at the per-user `~/.skrills/sync-state`.
    #[must_use]
    pub fn with_default_sync_state(self) -> Self {
        self.with_state_root(crate::conflict::default_root())
    }

    /// Lets `params.interactive` ask `decider` about each new or changed
    /// item before anything is written.
    #[must_use]
    pub fn with_decider(mut self, decider: Arc<dyn ItemDecider>) -> Self {
        self.decider = Some(decider);
        self
    }

    /// Consults the kill-switch marker at `path` instead of the per-user one;
    /// `None` consults none.
    #[must_use]
    pub fn with_kill_switch_marker(mut self, path: Option<std::path::PathBuf>) -> Self {
        self.kill_switch_marker = path;
        self
    }

    /// Snapshots what each non-dry-run sync overwrites or deletes under
    /// `root`, for `skrills sync-rollback`; `None` takes no snapshots.
    #[must_use]
    pub fn with_snapshot_root(mut self, root: Option<std::path::PathBuf>) -> Self {
        self.snapshot_root = root;
        self
    }

    /// [`Self::with_snapshot_root`] at [`crate::snapshot::default_root`].
    #[must_use]
    pub fn with_default_snapshots(self) -> Self {
        self.with_snapshot_root(crate::snapshot::default_root())
    }

    /// Performs the sync operation.
    ///
    /// Logs [`crate::adapters::traits::FieldSupport`] mismatches for observability but always delegates
    /// to the target adapter, adapters may implement creative mappings for
    /// fields they don't "natively" support (e.g., Copilot maps commands to
    /// prompts, Codex converts agents to skills).
    ///
    /// With `params.interactive`, every question is asked before the first
    /// write, so cancelling writes nothing.
    pub fn sync(&self, params: &SyncParams) -> Result<SyncReport> {
        // A dry run writes nothing, so there is nothing to confirm.
        let decider = if params.interactive && !params.dry_run {
            let Some(decider) = self.decider.as_deref() else {
                bail!("interactive sync needs an item decider; nothing was written");
            };
            Some(decider)
        } else {
            None
        };

        // A `skrills cold-window` in another process may have engaged its
        // kill switch (FR12). A dry run writes nothing, so it still runs.
        if !params.dry_run {
            crate::kill_switch_marker::ensure_not_engaged(self.kill_switch_marker.as_deref())?;
        }

        // Held for the whole run: the phases read, merge and rewrite shared
        // config files, and a second sync into the same target in between
        // would lose one of the updates. A dry run writes nothing.
        let _lock = if params.dry_run {
            None
        } else {
            Some(lock_target(&self.target.config_root())?)
        };

        // After the checks above so a refused sync leaves no snapshot, and
        // after the lock so the prior state is read once no other sync can
        // change it. A caller that already holds a session, such as
        // `sync-all` covering every target, gets one snapshot for the lot.
        let _snapshot = match (&self.snapshot_root, params.dry_run) {
            (Some(root), false) => crate::snapshot::begin_unless_active(root),
            _ => None,
        };

        // Read once, before any phase writes, so every pass compares against
        // the state the last sync left.
        let ledger = Ledger::load(
            self.state_root.as_deref(),
            crate::conflict::target_key(self.target.name(), &self.target.config_root()),
        );
        let Some(decider) = decider else {
            return self.run_phases(params, &Gate::off(), &ledger);
        };
        let book = DecisionBook::default();
        drop(self.run_phases(params, &Gate::collect(decider, &book), &ledger)?);
        self.run_phases(params, &Gate::apply(&book), &ledger)
    }

    /// Runs every phase once. `gate` decides what each writer is given: all of
    /// it when off, nothing (the questions are only asked) when collecting,
    /// the accepted items when applying.
    fn run_phases(&self, params: &SyncParams, gate: &Gate, ledger: &Ledger) -> Result<SyncReport> {
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
                // Only the writer's own directory: plugin-cache and
                // marketplace commands are never overwritten, so they
                // cannot make a command "already exist".
                let read_existing = || -> Result<Vec<Command>> {
                    let mut existing = self.target.read_commands(false)?;
                    existing.retain(|c| c.plugin_origin.is_none());
                    Ok(existing)
                };
                // A skipped existing command is never overwritten, so it
                // cannot be a conflict either.
                if params.dry_run && (params.force || !params.skip_existing_commands) {
                    self.scan_files(ledger, ItemKind::Command, &commands, read_existing);
                }
                *slot = sync_items(
                    commands,
                    params.force,
                    params.dry_run,
                    params.skip_existing_commands,
                    read_existing,
                    |items| {
                        self.write_files_gated(
                            gate,
                            ledger,
                            ItemKind::Command,
                            items.to_vec(),
                            read_existing,
                            |kept| self.target.write_commands(kept),
                        )
                    },
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
                let mut in_mirror_names = Vec::new();
                if mirror_carries_plugin_skills {
                    let before = skills.len();
                    in_mirror_names = skills
                        .iter()
                        .filter(|s| {
                            s.plugin_origin
                                .as_ref()
                                .is_some_and(|o| !params.is_plugin_excluded(&o.plugin_name))
                        })
                        .map(crate::adapters::utils::skill_folder_name)
                        .collect();
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
                    *slot = self.write_files_gated(
                        gate,
                        ledger,
                        ItemKind::Skill,
                        skills,
                        || self.target.read_skills(),
                        |kept| self.target.write_skills(kept),
                    )?;
                } else {
                    self.scan_files(ledger, ItemKind::Skill, &skills, || {
                        self.target.read_skills()
                    });
                    *slot = self.target.preview_skills(&skills)?;
                }
                // The flat copies go only because the plugin mirror carries
                // those skills; a rejected mirror leaves them as the only copy.
                let mirror_rejected =
                    gate.rejected(ItemKind::PluginAssets, ItemKind::PluginAssets.plural());
                if !params.keep_versions && !in_mirror_names.is_empty() && !mirror_rejected {
                    let removed = self.target.remove_flat_skill_copies(
                        &in_mirror_names,
                        params.dry_run || gate.is_collecting(),
                    )?;
                    slot.warnings.extend(removed.warnings);
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
                        *slot = self.write_servers_gated(gate, ledger, servers)?;
                    } else {
                        // A target the real run would refuse fails the preview too.
                        let target = self.target.read_mcp_servers()?;
                        for (name, server) in &servers {
                            ledger.server_conflict(name, server, target.get(name));
                        }
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
                        let accepted = match prefs.model.as_deref() {
                            Some(model) if !gate.is_off() => {
                                let current = self.target.read_preferences()?.model;
                                gate.accept_model(current.as_deref(), model)?
                            }
                            _ => true,
                        };
                        if gate.is_collecting() {
                            // Asked above; the applying pass writes.
                        } else if accepted {
                            *slot = self.target.write_preferences(&prefs)?;
                        } else {
                            slot.skipped.push(SkipReason::Declined {
                                item: "model".to_string(),
                            });
                        }
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
                    *slot = self.write_files_gated(
                        gate,
                        ledger,
                        ItemKind::Agent,
                        agents,
                        || self.target.read_agents(),
                        |kept| self.target.write_agents(kept),
                    )?;
                } else {
                    self.scan_files(ledger, ItemKind::Agent, &agents, || {
                        self.target.read_agents()
                    });
                    *slot = preview_items(&agents, || self.target.read_agents());
                }
                Ok(())
            })?;
        }

        // Sync hooks (lifecycle events). Cursor reads Claude's own hooks from
        // ~/.claude/settings.json and runs every match from both files, so a
        // copy would run each hook twice.
        if params.sync_hooks && source_name == "claude" && target_name == "cursor" {
            report.hooks.skipped.push(SkipReason::AgentSpecificFeature {
                item: "hooks".to_string(),
                feature: "Cursor loads Claude Code hooks from ~/.claude/settings.json itself"
                    .to_string(),
                suggestion: "Keep them in ~/.claude/settings.json; a copy in \
                             ~/.cursor/hooks.json would run each hook twice"
                    .to_string(),
            });
        } else if params.sync_hooks
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
                    *slot = self.write_files_gated(
                        gate,
                        ledger,
                        ItemKind::Hook,
                        hooks,
                        || self.target.read_hooks(),
                        |kept| self.target.write_hooks(kept),
                    )?;
                } else {
                    self.scan_files(ledger, ItemKind::Hook, &hooks, || self.target.read_hooks());
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
                    let read_existing = || self.target.read_instructions();
                    if params.dry_run && (params.force || !params.skip_existing_instructions) {
                        self.scan_files(
                            ledger,
                            ItemKind::Instruction,
                            &instructions,
                            read_existing,
                        );
                    }
                    *slot = sync_items(
                        instructions,
                        params.force,
                        params.dry_run,
                        params.skip_existing_instructions,
                        read_existing,
                        |items| {
                            self.write_files_gated(
                                gate,
                                ledger,
                                ItemKind::Instruction,
                                items.to_vec(),
                                read_existing,
                                |kept| self.target.write_instructions(kept),
                            )
                        },
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
                            let options = crate::adapters::MirrorOptions {
                                keep_versions: params.keep_versions,
                            };
                            // The mirror prunes as well as writes and has no
                            // per-file round trip, so it is one question.
                            let accepted = if gate.is_off() {
                                true
                            } else {
                                let pending = self.target.preview_plugin_assets(&assets)?.written;
                                pending == 0
                                    || gate.accept_category(
                                        ItemKind::PluginAssets,
                                        &[format!(
                                            "{pending} plugin asset file(s) to write or prune"
                                        )],
                                        0,
                                    )?
                            };
                            if gate.is_collecting() {
                                // Asked above; the applying pass writes.
                            } else if accepted {
                                *slot = self.target.write_plugin_assets_with(&assets, &options)?;
                            } else {
                                slot.skipped.push(SkipReason::Declined {
                                    item: ItemKind::PluginAssets.plural().to_string(),
                                });
                            }
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
        // A plain run overwrites what the baseline says the target changed;
        // an interactive one asked, so the person already knows.
        if gate.is_off() {
            self.warn_of_conflicts(params, ledger, &mut report);
        }
        if !params.dry_run && !gate.is_collecting() && !failed {
            let saved = ledger.persist(std::time::SystemTime::now(), |kind, name| {
                gate.answered(kind, name)
            });
            if let Err(e) = saved {
                tracing::warn!(error = %format!("{e:#}"), "Could not save the conflict baseline");
                // The report has no list of its own, so this rides on the
                // first phase.
                report.commands.warnings.push(format!(
                    "The sync worked, but its conflict baseline could not be saved: {e:#}"
                ));
            }
        }
        report.summary = report.format_summary(self.source.name(), self.target.name());

        Ok(report)
    }

    /// Adds, per kind, one warning naming the items both sides changed since
    /// the last sync. The source wins in a plain run, so the target's edits
    /// are gone unless a snapshot can bring them back.
    fn warn_of_conflicts(&self, params: &SyncParams, ledger: &Ledger, report: &mut SyncReport) {
        for kind in [
            ItemKind::Command,
            ItemKind::Skill,
            ItemKind::Agent,
            ItemKind::Hook,
            ItemKind::Instruction,
            ItemKind::McpServer,
        ] {
            let names = ledger.found_names(kind);
            if names.is_empty() {
                continue;
            }
            let label = if names.len() == 1 {
                kind.to_string()
            } else {
                format!("{} {}", names.len(), kind.plural())
            };
            let outcome = match (params.dry_run, crate::write_session::is_active()) {
                (true, _) => "a sync would overwrite the target's edits",
                (false, true) => {
                    "the source overwrote the target's edits; `skrills sync-rollback` undoes it"
                }
                (false, false) => "the source overwrote the target's edits",
            };
            let slot = match kind {
                ItemKind::Command => &mut report.commands,
                ItemKind::Skill => &mut report.skills,
                ItemKind::Agent => &mut report.agents,
                ItemKind::Hook => &mut report.hooks,
                ItemKind::Instruction => &mut report.instructions,
                ItemKind::McpServer => &mut report.mcp_servers,
                ItemKind::Preferences => &mut report.preferences,
                ItemKind::PluginAssets => &mut report.plugin_assets,
            };
            slot.warnings.push(format!(
                "{label} changed on both the source and the target since the last sync, so {outcome}: {}",
                names.join(", ")
            ));
        }
    }

    /// Finds the items the source and the target both changed, for a run that
    /// asks nobody (a plain sync or a dry run). A target that cannot be read
    /// reports none: the write reports the real failure.
    fn scan_files(
        &self,
        ledger: &Ledger,
        kind: ItemKind,
        items: &[Command],
        read_target: impl FnOnce() -> Result<Vec<Command>>,
    ) {
        if !ledger.has_baseline() {
            return;
        }
        let held = match read_target() {
            Ok(held) => held,
            Err(e) => {
                tracing::debug!(error = %format!("{e:#}"), "Could not read the target to look for conflicts");
                return;
            }
        };
        let by_key: HashMap<String, &Command> = held
            .iter()
            .filter(|t| t.plugin_origin.is_none())
            .map(|t| (file_key(kind, t), t))
            .collect();
        for item in items {
            ledger.file_conflict(kind, item, by_key.get(&file_key(kind, item)).copied());
        }
    }

    /// Notes what each item of `items` hashed to on both sides after the
    /// write, so the next sync can tell what changed. `skip` leaves out items
    /// whose baseline should not move.
    fn record_files(
        &self,
        ledger: &Ledger,
        kind: ItemKind,
        items: &[Command],
        read_target: impl FnOnce() -> Result<Vec<Command>>,
        skip: impl Fn(&Command) -> bool,
    ) {
        if !ledger.is_enabled() {
            return;
        }
        let held = match read_target() {
            Ok(held) => held,
            Err(e) => {
                tracing::debug!(error = %format!("{e:#}"), "Could not read the target to record a baseline");
                return;
            }
        };
        let by_key: HashMap<String, &Command> = held
            .iter()
            .filter(|t| t.plugin_origin.is_none())
            .map(|t| (file_key(kind, t), t))
            .collect();
        for item in items.iter().filter(|item| !skip(item)) {
            if let Some(after) = by_key.get(&file_key(kind, item)) {
                ledger.record_file(kind, item, after);
            }
        }
    }

    /// Hands `write` what `gate` allows: everything when off, nothing when
    /// collecting (the questions are asked and the target untouched), and the
    /// accepted items when applying.
    ///
    /// A category the target's writer does not give back byte for byte
    /// ([`AgentAdapter::decision_scope`]) is one question, and a rejection
    /// skips `write` altogether so none of its sweeps can run.
    fn write_files_gated(
        &self,
        gate: &Gate,
        ledger: &Ledger,
        kind: ItemKind,
        items: Vec<Command>,
        read_target: impl Fn() -> Result<Vec<Command>>,
        write: impl FnOnce(&[Command]) -> Result<WriteReport>,
    ) -> Result<WriteReport> {
        if gate.is_off() {
            self.scan_files(ledger, kind, &items, &read_target);
            let report = write(&items)?;
            self.record_files(ledger, kind, &items, &read_target, |_| false);
            return Ok(report);
        }
        // An unreadable target cannot be classified, so it is not written.
        let mut target = read_target()?;
        target.retain(|t| t.plugin_origin.is_none());
        let resolved = match self.target.decision_scope(kind) {
            DecisionScope::PerItem => {
                gate.resolve_files(kind, items.clone(), &target, |c| file_key(kind, c), ledger)?
            }
            DecisionScope::WholeCategory => {
                let pending = changed_names(&items, &target, |c| file_key(kind, c));
                // The target's own preview is the arbiter where it differs
                // from a byte compare (Codex rewrites a skill's `name`).
                let anything = match kind {
                    ItemKind::Skill => self.target.preview_skills(&items)?.written > 0,
                    _ => !pending.is_empty(),
                };
                let conflicting = items
                    .iter()
                    .filter(|item| {
                        let by_key = file_key(kind, item);
                        let held = target.iter().find(|t| file_key(kind, t) == by_key);
                        ledger.file_conflict(kind, item, held)
                    })
                    .count();
                if anything && !gate.accept_category(kind, &pending, conflicting)? {
                    Resolved {
                        items: Vec::new(),
                        declined: vec![kind.plural().to_string()],
                        skip_write: true,
                    }
                } else {
                    Resolved {
                        items: items.clone(),
                        declined: Vec::new(),
                        skip_write: false,
                    }
                }
            }
        };
        if gate.is_collecting() {
            return Ok(WriteReport::default());
        }
        let report = if resolved.skip_write {
            WriteReport::default()
        } else {
            write(&resolved.items)?
        };
        // A rejected change leaves its baseline where it was, except a
        // rejected conflict: that was resolved, and asking again every sync
        // would be noise.
        let declined = |item: &Command| {
            resolved
                .declined
                .iter()
                .any(|name| name == &item.name || name == kind.plural())
                && !ledger.is_found(kind, &item.name)
        };
        self.record_files(ledger, kind, &items, &read_target, declined);
        Ok(gate.annotate(report, &resolved.declined))
    }

    /// [`Self::write_files_gated`] for MCP servers, which are keyed by name
    /// and merged into the target's config.
    fn write_servers_gated(
        &self,
        gate: &Gate,
        ledger: &Ledger,
        servers: HashMap<String, crate::common::McpServer>,
    ) -> Result<WriteReport> {
        if gate.is_off() {
            if ledger.has_baseline() {
                match self.target.read_mcp_servers() {
                    Ok(held) => {
                        for (name, server) in &servers {
                            ledger.server_conflict(name, server, held.get(name));
                        }
                    }
                    Err(e) => tracing::debug!(
                        error = %format!("{e:#}"),
                        "Could not read the target to look for conflicts"
                    ),
                }
            }
            let report = self.target.write_mcp_servers(&servers)?;
            self.record_servers(ledger, &servers, &[]);
            return Ok(report);
        }
        let held = self.target.read_mcp_servers()?;
        let scope = self.target.decision_scope(ItemKind::McpServer);
        let source = servers.clone();
        let (kept, declined) = gate.resolve_servers(scope, servers, &held, ledger)?;
        if gate.is_collecting() {
            return Ok(WriteReport::default());
        }
        let report = if kept.is_empty() && !declined.is_empty() {
            WriteReport::default()
        } else {
            self.target.write_mcp_servers(&kept)?
        };
        self.record_servers(ledger, &source, &declined);
        Ok(gate.annotate(report, &declined))
    }

    /// [`Self::record_files`] for MCP servers; `declined` names keep their
    /// baseline unless they were a conflict.
    fn record_servers(
        &self,
        ledger: &Ledger,
        servers: &HashMap<String, crate::common::McpServer>,
        declined: &[String],
    ) {
        if !ledger.is_enabled() {
            return;
        }
        let held = match self.target.read_mcp_servers() {
            Ok(held) => held,
            Err(e) => {
                tracing::debug!(error = %format!("{e:#}"), "Could not read the target to record a baseline");
                return;
            }
        };
        for (name, server) in servers {
            let moved = !declined.contains(name) || ledger.is_found(ItemKind::McpServer, name);
            if let (true, Some(after)) = (moved, held.get(name)) {
                ledger.record_server(name, server, after);
            }
        }
    }
}

/// Runs a sync between two named platforms using `create_adapter`.
///
/// This avoids the combinatorial match arm explosion that occurs when each
/// (from, to) pair is constructed explicitly.
pub fn sync_between(
    from: crate::Platform,
    to: crate::Platform,
    params: &SyncParams,
) -> Result<SyncReport> {
    sync_between_endpoints(&from.into(), &to.into(), params)
}

/// [`sync_between`] that answers `params.interactive` with `decider`.
pub fn sync_between_with_decider(
    from: crate::Platform,
    to: crate::Platform,
    params: &SyncParams,
    decider: Arc<dyn ItemDecider>,
) -> Result<SyncReport> {
    sync_between_endpoints_with_decider(&from.into(), &to.into(), params, decider)
}

/// [`sync_between`] for endpoints, which may be configured adapters.
pub fn sync_between_endpoints(
    from: &crate::SyncEndpoint,
    to: &crate::SyncEndpoint,
    params: &SyncParams,
) -> Result<SyncReport> {
    let source = create_endpoint_source_adapter(from, params)?;
    let target = create_endpoint_adapter(to)?;
    SyncOrchestrator::new(source, target)
        .with_default_snapshots()
        .with_default_sync_state()
        .sync(params)
}

/// [`sync_between_endpoints`] that answers `params.interactive` with `decider`.
pub fn sync_between_endpoints_with_decider(
    from: &crate::SyncEndpoint,
    to: &crate::SyncEndpoint,
    params: &SyncParams,
    decider: Arc<dyn ItemDecider>,
) -> Result<SyncReport> {
    let source = create_endpoint_source_adapter(from, params)?;
    let target = create_endpoint_adapter(to)?;
    SyncOrchestrator::new(source, target)
        .with_default_snapshots()
        .with_default_sync_state()
        .with_decider(decider)
        .sync(params)
}

/// [`create_adapter`] for an endpoint. A custom name that is not configured,
/// or whose file is invalid, is an error that says what exists or what is
/// wrong, before anything is read or written.
pub fn create_endpoint_adapter(endpoint: &crate::SyncEndpoint) -> Result<Box<dyn AgentAdapter>> {
    match endpoint {
        crate::SyncEndpoint::Builtin(platform) => create_adapter(*platform),
        crate::SyncEndpoint::Custom(name) => {
            let home = crate::adapter_config::user_home()?;
            let config = crate::adapter_config::load_adapter(name, &home)?;
            Ok(Box::new(crate::adapters::ConfigAdapter::new(config)))
        }
    }
}

/// [`create_source_adapter`] for an endpoint.
pub fn create_endpoint_source_adapter(
    endpoint: &crate::SyncEndpoint,
    params: &SyncParams,
) -> Result<Box<dyn AgentAdapter>> {
    match endpoint {
        crate::SyncEndpoint::Builtin(platform) => create_source_adapter(*platform, params),
        crate::SyncEndpoint::Custom(_) => create_endpoint_adapter(endpoint),
    }
}

/// [`create_adapter`] for the side being read: a Claude source also reads
/// marketplace skills when `params.include_marketplace` asks for them, as it
/// already does for commands.
pub fn create_source_adapter(
    platform: crate::Platform,
    params: &SyncParams,
) -> Result<Box<dyn AgentAdapter>> {
    if platform == crate::Platform::Claude {
        return Ok(Box::new(
            crate::adapters::ClaudeAdapter::new()?
                .with_marketplace_skills(params.include_marketplace),
        ));
    }
    create_adapter(platform)
}

/// Syncs the skills under `claude_root` into `home/.agents/skills`, the
/// directory Codex loads user skills from, with `include_marketplace` adding
/// skills of marketplace plugins that are not installed.
///
/// Commands, MCP servers and the rest are left to [`sync_between`]; this is
/// the skills-only direction `sync`, `mirror` and `setup` run.
pub fn sync_claude_skills_to_codex(
    claude_root: &std::path::Path,
    home: &std::path::Path,
    include_marketplace: bool,
    dry_run: bool,
) -> Result<SyncReport> {
    let source = crate::adapters::ClaudeAdapter::with_root(claude_root.to_path_buf())
        .with_marketplace_skills(include_marketplace);
    let target = crate::adapters::CodexAdapter::with_home(home);
    let params = SyncParams {
        from: Some("claude".to_string()),
        dry_run,
        sync_skills: true,
        sync_commands: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_hooks: false,
        sync_instructions: false,
        sync_plugin_assets: false,
        include_marketplace,
        ..Default::default()
    };
    SyncOrchestrator::new(source, target)
        .with_snapshot_root(Some(home.join(".skrills").join("snapshots")))
        .sync(&params)
}

/// Creates an adapter for the given platform name.
///
/// Returns a boxed `AgentAdapter` for the specified platform.
pub fn create_adapter(platform: crate::Platform) -> Result<Box<dyn AgentAdapter>> {
    use crate::Platform;
    Ok(match platform {
        Platform::Claude => Box::new(crate::adapters::ClaudeAdapter::new()?),
        Platform::Codex => Box::new(crate::adapters::CodexAdapter::new()?),
        Platform::Copilot => Box::new(crate::adapters::CopilotAdapter::new()?),
        Platform::Cursor => Box::new(crate::adapters::CursorAdapter::new()?),
    })
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

    /// Skip-existing compared against plugin-cache commands too, which the
    /// Claude writer never overwrites, so a new command was skipped because a
    /// plugin happened to ship one with the same name.
    #[test]
    fn skip_existing_ignores_plugin_commands_the_writer_would_not_touch() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("prompts")).unwrap();
        fs::write(src_dir.path().join("prompts/deploy.md"), "# Deploy").unwrap();
        let plugin_cmds = tgt_dir
            .path()
            .join("plugins/cache/market/plug/1.0.0/commands");
        fs::create_dir_all(&plugin_cmds).unwrap();
        fs::write(plugin_cmds.join("deploy.md"), "# Plugin deploy").unwrap();

        let report = SyncOrchestrator::new(
            CodexAdapter::with_root(src_dir.path().to_path_buf()),
            ClaudeAdapter::with_root(tgt_dir.path().to_path_buf()),
        )
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

        assert_eq!(report.commands.written, 1, "{:?}", report.commands);
        assert!(tgt_dir.path().join("commands/deploy.md").exists());
    }

    /// A target config that does not parse makes the real run refuse to
    /// write; the dry run reported every server as "would write".
    #[test]
    fn dry_run_reports_an_unreadable_mcp_target_like_the_real_run() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::write(
            src_dir.path().join(".claude.json"),
            r#"{"mcpServers": {"s": {"command": "/bin/s"}}}"#,
        )
        .unwrap();
        fs::write(tgt_dir.path().join("config.toml"), "[unclosed\n").unwrap();

        let report = SyncOrchestrator::new(
            ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
            CodexAdapter::with_root(tgt_dir.path().to_path_buf()),
        )
        .sync(&SyncParams {
            dry_run: true,
            sync_commands: false,
            sync_mcp_servers: true,
            sync_preferences: false,
            sync_skills: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: false,
            sync_plugin_assets: false,
            ..Default::default()
        })
        .unwrap();

        assert_eq!(report.mcp_servers.written, 0, "{:?}", report.mcp_servers);
        assert!(!report.success);
    }

    /// Two syncs into one target read, merge and write the same config files;
    /// without a lock one of them loses its update. A second sync waits for
    /// the lock on the target root.
    #[test]
    fn a_sync_waits_while_another_holds_the_target_lock() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("commands")).unwrap();
        fs::write(src_dir.path().join("commands/hello.md"), "# Hello").unwrap();
        let held = fs::File::create(tgt_dir.path().join(SYNC_LOCK_FILE)).unwrap();
        held.lock().unwrap();

        let (done, finished) = std::sync::mpsc::channel();
        let (src, tgt) = (src_dir.path().to_path_buf(), tgt_dir.path().to_path_buf());
        let worker = std::thread::spawn(move || {
            let report =
                SyncOrchestrator::new(ClaudeAdapter::with_root(src), CodexAdapter::with_root(tgt))
                    .sync(&SyncParams {
                        sync_commands: true,
                        sync_mcp_servers: false,
                        sync_preferences: false,
                        sync_skills: false,
                        sync_agents: false,
                        sync_hooks: false,
                        sync_instructions: false,
                        sync_plugin_assets: false,
                        ..Default::default()
                    })
                    .map(|r| r.commands.written);
            done.send(report).unwrap();
        });

        assert!(
            finished
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "the sync ran while the lock was held"
        );
        held.unlock().unwrap();
        assert_eq!(finished.recv().unwrap().unwrap(), 1);
        worker.join().unwrap();
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
        let settings_path = src_dir.path().join(".claude.json");
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

        // Codex reads MCP servers from config.toml.
        let tgt_config = fs::read_to_string(tgt_dir.path().join("config.toml")).unwrap();
        assert!(
            tgt_config.contains("[mcp_servers.test-server]"),
            "{tgt_config}"
        );
    }

    #[test]
    fn sync_keeps_the_codex_model_for_a_claude_model() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();

        // Create source settings with Claude model
        let settings_path = src_dir.path().join("settings.json");
        fs::write(&settings_path, r#"{"model": "sonnet"}"#).unwrap();
        fs::write(
            tgt_dir.path().join("config.toml"),
            "model = \"gpt-6.1-sol\"\n",
        )
        .unwrap();

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
        assert_eq!(report.preferences.written, 0);

        // No current Claude -> Codex model table, so Codex keeps its model
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());
        assert_eq!(
            target.read_preferences().unwrap().model.as_deref(),
            Some("gpt-6.1-sol")
        );
        assert!(!tgt_dir.path().join("config.json").exists());
    }

    #[test]
    fn sync_leaves_the_claude_model_unset_for_an_openai_model() {
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
        assert_eq!(report.preferences.written, 0);

        // No OpenAI -> Claude model table, so Claude's settings are untouched
        assert!(!tgt_dir.path().join("settings.json").exists());
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
        fs::write(tgt_dir.path().join("config.toml"), "model = \"gpt-5\"\n").unwrap();

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
        let content = fs::read_to_string(tgt_dir.path().join("config.toml")).unwrap();
        assert_eq!(content, "model = \"gpt-5\"\n");
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
            src_dir.path().join(".claude.json"),
            r#"{"mcpServers": {"s": {"command": "/bin/s"}}}"#,
        )
        .unwrap();
        // A malformed config.toml makes the MCP writer fail.
        fs::write(tgt_dir.path().join("config.toml"), "[unclosed\n").unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(tgt_dir.path().to_path_buf());
        let report = SyncOrchestrator::new(source, target)
            .sync(&SyncParams {
                sync_commands: true,
                sync_mcp_servers: true,
                sync_preferences: false,
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
                .mcp_servers
                .warnings
                .iter()
                .any(|w| w.contains("failed")),
            "{:?}",
            report.mcp_servers.warnings
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
            r#"{"model": "sonnet"}"#,
        )
        .unwrap();
        fs::write(
            src_dir.path().join(".claude.json"),
            r#"{"mcpServers": {"s": {"command": "/bin/s", "args": ["-x"]}}}"#,
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
        // A Claude model has no Codex equivalent, so nothing to write there.
        assert_eq!(first.preferences.written, 0);

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

    /// Cursor loads Claude Code hooks from `~/.claude/settings.json` itself
    /// (cursor.com/docs/reference/third-party-hooks) and runs every match from
    /// both files, so a copy in `~/.cursor/hooks.json` ran each hook twice.
    #[test]
    fn claude_to_cursor_skips_hooks_cursor_loads_natively() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("hooks")).unwrap();
        fs::write(
            src_dir.path().join("hooks/PreToolUse.md"),
            r#"[{"command": "echo hi"}]"#,
        )
        .unwrap();

        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = crate::adapters::CursorAdapter::with_root(tgt_dir.path().to_path_buf());
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

        assert_eq!(report.hooks.written, 0, "{:?}", report.hooks);
        assert!(
            matches!(
                report.hooks.skipped.as_slice(),
                [SkipReason::AgentSpecificFeature { feature, .. }] if feature.contains("settings.json")
            ),
            "{:?}",
            report.hooks.skipped
        );
        assert!(!tgt_dir.path().join("hooks.json").exists());
        assert!(report.success);
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

    /// Runs a skills-and-mirror sync of the seeded root into a Cursor target
    /// whose flat `skills/deep-work/SKILL.md` already holds `flat_copy`.
    fn sync_over_flat_copy(
        flat_copy: &str,
        keep_versions: bool,
        dry_run: bool,
    ) -> (bool, SyncReport) {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        seed_claude_root_with_plugin_and_local_skill(src_dir.path());
        let flat = tgt_dir.path().join("skills/deep-work");
        fs::create_dir_all(&flat).unwrap();
        fs::write(flat.join("SKILL.md"), flat_copy).unwrap();

        let params = SyncParams {
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: false,
            full_plugin_mirror: true,
            keep_versions,
            dry_run,
            ..Default::default()
        };
        let source = ClaudeAdapter::with_root(src_dir.path().to_path_buf());
        let target = crate::adapters::CursorAdapter::with_root(tgt_dir.path().to_path_buf());
        let report = SyncOrchestrator::new(source, target).sync(&params).unwrap();
        (flat.exists(), report)
    }

    /// Builds before 0.9.1 copied plugin skills flat with the frontmatter
    /// stripped. Cursor cannot load those, and the mirror now carries the skill.
    #[test]
    fn full_mirror_sync_removes_a_legacy_flat_copy_of_a_plugin_skill() {
        let (exists, report) = sync_over_flat_copy("Old description\n# Deep work\n", false, false);
        assert!(!exists, "{:?}", report.skills.warnings);
    }

    #[test]
    fn full_mirror_sync_keeps_a_flat_skill_the_user_wrote() {
        let mine = "---\nname: deep-work\ndescription: Mine\n---\n# Mine\n";
        let (exists, report) = sync_over_flat_copy(mine, false, false);
        assert!(exists, "{:?}", report.skills.warnings);
    }

    #[test]
    fn full_mirror_sync_keeps_superseded_flat_copies_with_keep_versions_or_dry_run() {
        let legacy = "Old description\n# Deep work\n";
        assert!(sync_over_flat_copy(legacy, true, false).0);
        let (exists, report) = sync_over_flat_copy(legacy, false, true);
        assert!(exists, "a dry run removes nothing");
        assert!(
            report
                .skills
                .warnings
                .iter()
                .any(|w| w.contains("deep-work")),
            "the preview must name the copy it would remove: {:?}",
            report.skills.warnings
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
                .join(".agents/skills/agent-reviewer/SKILL.md")
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

    fn tree_of(root: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| entry.file_name() != SYNC_LOCK_FILE)
            .map(|entry| {
                let relative = entry.path().strip_prefix(root).unwrap().to_path_buf();
                (relative, fs::read(entry.path()).unwrap())
            })
            .collect()
    }

    fn deciding(
        answer: impl Fn(&crate::decision::ItemChange) -> crate::decision::Decision
            + Send
            + Sync
            + 'static,
    ) -> Arc<dyn ItemDecider> {
        Arc::new(move |change: &crate::decision::ItemChange| Ok(answer(change)))
    }

    /// Syncs the seeded plugin root into a Cursor target that still holds a
    /// legacy flat copy of the plugin skill, interactively.
    fn interactive_mirror_sync(
        answer: impl Fn(&crate::decision::ItemChange) -> crate::decision::Decision
            + Send
            + Sync
            + 'static,
    ) -> (tempfile::TempDir, SyncReport) {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        seed_claude_root_with_plugin_and_local_skill(src_dir.path());
        let flat = tgt_dir.path().join("skills/deep-work");
        fs::create_dir_all(&flat).unwrap();
        fs::write(flat.join("SKILL.md"), "Old description\n# Deep work\n").unwrap();

        let report = SyncOrchestrator::new(
            ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
            crate::adapters::CursorAdapter::with_root(tgt_dir.path().to_path_buf()),
        )
        .with_kill_switch_marker(None)
        .with_decider(deciding(answer))
        .sync(&SyncParams {
            interactive: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: false,
            full_plugin_mirror: true,
            ..Default::default()
        })
        .unwrap();
        (tgt_dir, report)
    }

    #[test]
    fn rejecting_everything_keeps_the_plugin_mirror_and_the_flat_copy_it_would_replace() {
        let before = {
            let probe = tempdir().unwrap();
            let flat = probe.path().join("skills/deep-work");
            fs::create_dir_all(&flat).unwrap();
            fs::write(flat.join("SKILL.md"), "Old description\n# Deep work\n").unwrap();
            tree_of(probe.path())
        };

        let (tgt_dir, report) = interactive_mirror_sync(|_| crate::decision::Decision::Reject);

        assert_eq!(tree_of(tgt_dir.path()), before);
        assert_eq!(report.plugin_assets.written, 0);
        assert!(report
            .plugin_assets
            .skipped
            .iter()
            .any(|r| matches!(r, SkipReason::Declined { item } if item == "plugin assets")));
    }

    #[test]
    fn rejecting_only_the_mirror_keeps_the_flat_copy_but_writes_what_was_accepted() {
        let (tgt_dir, _) = interactive_mirror_sync(|change| {
            if change.kind == ItemKind::PluginAssets {
                crate::decision::Decision::Reject
            } else {
                crate::decision::Decision::Accept
            }
        });

        assert!(tgt_dir.path().join("skills/deep-work/SKILL.md").exists());
        assert!(tgt_dir.path().join("skills/local-only/SKILL.md").exists());
        assert!(!tgt_dir.path().join("plugins").exists());
    }

    #[test]
    fn accepting_the_mirror_removes_the_flat_copy_as_a_default_sync_does() {
        let (tgt_dir, report) = interactive_mirror_sync(|_| crate::decision::Decision::Accept);

        assert!(!tgt_dir.path().join("skills/deep-work").exists());
        assert!(tgt_dir
            .path()
            .join("plugins/local/my-plugin/skills/deep-work/SKILL.md")
            .exists());
        assert!(report.success, "{report:?}");
    }

    /// Hooks are rewritten into the target's JSON, so they are one question,
    /// and a rejection must not reach the writer at all.
    #[test]
    fn rejecting_the_hooks_never_calls_the_hook_writer() {
        use crate::adapters::traits::{FieldSupport, MockAgentAdapter};
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("hooks")).unwrap();
        fs::write(src_dir.path().join("hooks/PreToolUse.md"), "echo hi").unwrap();
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = asked.clone();

        let mut target = MockAgentAdapter::new();
        target.expect_name().return_const("mock".to_string());
        let root = tgt_dir.path().to_path_buf();
        target.expect_config_root().returning(move || root.clone());
        target.expect_write_support().returning(|| FieldSupport {
            hooks: true,
            ..FieldSupport::default()
        });
        target.expect_read_hooks().returning(|| Ok(Vec::new()));
        target
            .expect_decision_scope()
            .returning(|_| DecisionScope::WholeCategory);
        target.expect_write_hooks().times(0);

        let report = SyncOrchestrator::new(
            ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
            target,
        )
        .with_kill_switch_marker(None)
        .with_decider(Arc::new(move |change: &crate::decision::ItemChange| {
            record.lock().unwrap().push(change.clone());
            Ok(crate::decision::Decision::Reject)
        }))
        .sync(&SyncParams {
            interactive: true,
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

        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 1, "{asked:?}");
        assert_eq!(asked[0].kind, ItemKind::Hook);
        assert_eq!(asked[0].diff, "PreToolUse");
        assert_eq!(report.hooks.written, 0);
        assert!(matches!(
            report.hooks.skipped.as_slice(),
            [SkipReason::Declined { item }] if item == "hooks"
        ));
    }

    #[test]
    fn accepting_the_hooks_calls_the_hook_writer_once_with_all_of_them() {
        use crate::adapters::traits::{FieldSupport, MockAgentAdapter};
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("hooks")).unwrap();
        fs::write(src_dir.path().join("hooks/PreToolUse.md"), "echo hi").unwrap();

        let mut target = MockAgentAdapter::new();
        target.expect_name().return_const("mock".to_string());
        let root = tgt_dir.path().to_path_buf();
        target.expect_config_root().returning(move || root.clone());
        target.expect_write_support().returning(|| FieldSupport {
            hooks: true,
            ..FieldSupport::default()
        });
        target.expect_read_hooks().returning(|| Ok(Vec::new()));
        target
            .expect_decision_scope()
            .returning(|_| DecisionScope::WholeCategory);
        target
            .expect_write_hooks()
            .times(1)
            .withf(|hooks| hooks.len() == 1 && hooks[0].name == "PreToolUse")
            .returning(|_| {
                Ok(WriteReport {
                    written: 1,
                    ..WriteReport::default()
                })
            });

        let report = SyncOrchestrator::new(
            ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
            target,
        )
        .with_kill_switch_marker(None)
        .with_decider(deciding(|_| crate::decision::Decision::Accept))
        .sync(&SyncParams {
            interactive: true,
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

        assert_eq!(report.hooks.written, 1);
    }

    #[test]
    fn create_adapter_all_platforms() {
        // These only verify the adapter constructs; the actual root may not exist
        // on the test machine, but with_root tests cover that path.
        for platform in crate::Platform::ALL {
            assert_eq!(create_adapter(platform).unwrap().name(), platform.as_str());
        }
    }

    /// A sync with a snapshot root: the source command and the target prompt
    /// it overwrites, plus a second command the target has never had.
    fn snapshotted_sync(
        root: &std::path::Path,
        dry_run: bool,
    ) -> (tempfile::TempDir, std::path::PathBuf) {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("commands")).unwrap();
        fs::write(src_dir.path().join("commands/hello.md"), "# New Hello").unwrap();
        fs::write(src_dir.path().join("commands/fresh.md"), "# Fresh").unwrap();
        fs::create_dir_all(tgt_dir.path().join("prompts")).unwrap();
        fs::write(tgt_dir.path().join("prompts/hello.md"), "# Old Hello").unwrap();

        SyncOrchestrator::new(
            ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
            CodexAdapter::with_root(tgt_dir.path().to_path_buf()),
        )
        .with_snapshot_root(Some(root.to_path_buf()))
        .sync(&SyncParams {
            dry_run,
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_skills: false,
            ..Default::default()
        })
        .unwrap()
        .success
        .then_some(())
        .expect("sync succeeds");
        let target = tgt_dir.path().to_path_buf();
        (tgt_dir, target)
    }

    #[test]
    fn a_sync_over_an_existing_file_snapshots_its_old_bytes() {
        let snaps = tempdir().unwrap();
        let (_tgt, target) = snapshotted_sync(snaps.path(), false);

        assert_eq!(
            fs::read_to_string(target.join("prompts/hello.md")).unwrap(),
            "# New Hello"
        );
        let taken = crate::snapshot::list(snaps.path()).unwrap();
        assert_eq!(taken.len(), 1);
        let blob = snaps
            .path()
            .join(&taken[0].name)
            .join("blobs")
            .join(crate::adapters::utils::hash_content(b"# Old Hello"));
        assert_eq!(fs::read(blob).unwrap(), b"# Old Hello");
    }

    #[test]
    fn rollback_undoes_a_sync_by_restoring_old_bytes_and_deleting_new_files() {
        let snaps = tempdir().unwrap();
        let (_tgt, target) = snapshotted_sync(snaps.path(), false);
        assert!(target.join("prompts/fresh.md").exists());

        crate::snapshot::rollback(snaps.path(), None).unwrap();
        assert_eq!(
            fs::read_to_string(target.join("prompts/hello.md")).unwrap(),
            "# Old Hello"
        );
        assert!(!target.join("prompts/fresh.md").exists());
    }

    #[test]
    fn a_dry_run_takes_no_snapshot() {
        let snaps = tempdir().unwrap();
        let root = snaps.path().join("snapshots");
        let (_tgt, target) = snapshotted_sync(&root, true);

        assert!(!root.exists());
        assert_eq!(
            fs::read_to_string(target.join("prompts/hello.md")).unwrap(),
            "# Old Hello"
        );
    }

    #[test]
    fn a_sync_refused_before_it_starts_takes_no_snapshot() {
        let snaps = tempdir().unwrap();
        let root = snaps.path().join("snapshots");
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        let outcome = SyncOrchestrator::new(
            ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
            CodexAdapter::with_root(tgt_dir.path().to_path_buf()),
        )
        .with_snapshot_root(Some(root.clone()))
        .sync(&SyncParams {
            interactive: true,
            ..Default::default()
        });

        assert!(outcome.is_err());
        assert!(!root.exists());
    }

    #[test]
    fn a_sync_that_changes_nothing_leaves_no_snapshot() {
        let snaps = tempdir().unwrap();
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        let sync_once = || {
            SyncOrchestrator::new(
                ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
                CodexAdapter::with_root(tgt_dir.path().to_path_buf()),
            )
            .with_snapshot_root(Some(snaps.path().to_path_buf()))
            .sync(&SyncParams {
                sync_mcp_servers: false,
                sync_preferences: false,
                ..Default::default()
            })
            .unwrap()
            .success
            .then_some(())
            .expect("sync succeeds");
        };
        // The first sync creates the target's folders, which rollback must
        // be able to remove; the second finds them and changes nothing.
        sync_once();
        let first = crate::snapshot::list(snaps.path()).unwrap();
        assert_eq!(first.len(), 1);
        sync_once();

        assert_eq!(crate::snapshot::list(snaps.path()).unwrap(), first);
    }

    #[test]
    fn a_sync_without_a_snapshot_root_records_nothing_and_still_writes() {
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        fs::create_dir_all(src_dir.path().join("commands")).unwrap();
        fs::write(src_dir.path().join("commands/hello.md"), "# Hello").unwrap();
        let orchestrator = SyncOrchestrator::new(
            ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
            CodexAdapter::with_root(tgt_dir.path().to_path_buf()),
        );
        assert_eq!(orchestrator.snapshot_root, None);
        orchestrator
            .sync(&SyncParams {
                sync_mcp_servers: false,
                sync_preferences: false,
                sync_skills: false,
                ..Default::default()
            })
            .unwrap()
            .success
            .then_some(())
            .expect("sync succeeds");
        assert!(tgt_dir.path().join("prompts/hello.md").exists());
    }

    #[test]
    fn a_sync_inside_a_running_session_joins_it_instead_of_starting_another() {
        let snaps = tempdir().unwrap();
        let outer_root = snaps.path().join("outer");
        let inner_root = snaps.path().join("inner");
        let outer = crate::snapshot::begin(&outer_root).unwrap();
        let (_tgt, _target) = snapshotted_sync(&inner_root, false);
        drop(outer);

        assert!(!inner_root.exists());
        let joined = crate::snapshot::list(&outer_root).unwrap();
        assert_eq!(joined.len(), 1);
        assert_eq!(
            joined[0].entries, 2,
            "both of the sync's writes land in the outer snapshot"
        );
    }

    #[test]
    fn a_sync_still_writes_when_the_snapshot_root_cannot_be_created() {
        let tmp = tempdir().unwrap();
        let blocker = tmp.path().join("blocker");
        fs::write(&blocker, "a file where the snapshot root should go").unwrap();
        let (_tgt, target) = snapshotted_sync(&blocker.join("snapshots"), false);

        assert_eq!(
            fs::read_to_string(target.join("prompts/hello.md")).unwrap(),
            "# New Hello"
        );
    }

    #[test]
    fn an_interactive_sync_snapshots_what_the_user_accepted() {
        let _lock = skrills_test_utils::env_guard();
        let home = tempdir().unwrap();
        let _env = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        fs::create_dir_all(home.path().join(".claude/commands")).unwrap();
        fs::write(home.path().join(".claude/commands/hello.md"), "# New Hello").unwrap();
        fs::create_dir_all(home.path().join(".codex/prompts")).unwrap();
        fs::write(home.path().join(".codex/prompts/hello.md"), "# Old Hello").unwrap();

        let accept_all = |_: &crate::decision::ItemChange| Ok(crate::decision::Decision::Accept);
        let report = sync_between_with_decider(
            crate::Platform::Claude,
            crate::Platform::Codex,
            &SyncParams {
                interactive: true,
                sync_commands: true,
                sync_mcp_servers: false,
                sync_preferences: false,
                sync_skills: false,
                ..Default::default()
            },
            Arc::new(accept_all),
        )
        .unwrap();

        assert!(report.success);
        let snaps = home.path().join(".skrills/snapshots");
        let taken = crate::snapshot::list(&snaps).unwrap();
        assert_eq!(taken.len(), 1, "the accepted overwrite is snapshotted");
        let blob = snaps
            .join(&taken[0].name)
            .join("blobs")
            .join(crate::adapters::utils::hash_content(b"# Old Hello"));
        assert_eq!(fs::read(blob).unwrap(), b"# Old Hello");
    }

    #[test]
    fn default_snapshots_point_at_the_per_user_snapshot_directory() {
        let _lock = skrills_test_utils::env_guard();
        let home = tempdir().unwrap();
        let _env = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        let src_dir = tempdir().unwrap();
        let tgt_dir = tempdir().unwrap();
        let orchestrator = SyncOrchestrator::new(
            ClaudeAdapter::with_root(src_dir.path().to_path_buf()),
            CodexAdapter::with_root(tgt_dir.path().to_path_buf()),
        )
        .with_default_snapshots();
        assert_eq!(
            orchestrator.snapshot_root,
            Some(home.path().join(".skrills").join("snapshots"))
        );
    }
}
