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
//! - **Skills**: Cursor skills have no YAML frontmatter, Claude frontmatter is stripped
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

    /// Fields Cursor handles in at least one direction. The two directional
    /// methods narrow this: only `plugin_assets` differs between them.
    fn field_support() -> FieldSupport {
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
}

impl AgentAdapter for CursorAdapter {
    fn name(&self) -> &str {
        "cursor"
    }

    fn config_root(&self) -> PathBuf {
        self.root.clone()
    }

    /// Cursor receives the plugin mirror and is never a source for it:
    /// `read_plugin_assets` is not implemented, and the trait default's empty
    /// result is indistinguishable from a source that genuinely had none.
    fn read_support(&self) -> FieldSupport {
        FieldSupport {
            plugin_assets: false,
            ..Self::field_support()
        }
    }

    fn write_support(&self) -> FieldSupport {
        Self::field_support()
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

    /// Mirrors plugin content into `~/.cursor/plugins/local/<plugin>/`,
    /// preserving each file's path within the plugin.
    ///
    /// Carries skill bodies and runtime scripts when the reader ran with
    /// `full_mirror = true`. The orchestrator then drops those same skills from
    /// the flat `~/.cursor/skills` copy so nothing is written twice, while
    /// skills that come from no plugin keep going there. A batch that carries no
    /// plugin manifest is refused per plugin: Cursor cannot load a
    /// `plugins/local/<p>/` without one.
    ///
    /// Claude's `.claude-plugin/` manifest directory is renamed to the
    /// `.cursor-plugin/` name Cursor reads.
    fn write_plugin_assets(&self, assets: &[PluginAsset]) -> Result<WriteReport> {
        crate::adapters::utils::ensure_not_engaged(self.kill_switch.as_ref())?;
        use std::collections::HashSet;
        use std::fs;
        use std::path::PathBuf;

        let mut report = WriteReport::default();

        if assets.is_empty() {
            return Ok(report);
        }

        let local_dir = self.root.join("plugins").join("local");
        fs::create_dir_all(&local_dir)?;

        let plan = plan_mirror(&local_dir, assets);
        report.warnings.extend(plan.refusals);

        // Every accepted plugin, whether or not its batch was complete. Pruning
        // must not remove a directory this run just wrote into.
        let mut touched: HashSet<String> = HashSet::new();

        for (dir_name, batch) in &plan.accepted {
            let plugin_dir = local_dir.join(dir_name);
            if let Err(msg) = prepare_plugin_dir(&plugin_dir) {
                tracing::warn!(plugin = %dir_name, reason = %msg, "Refusing plugin mirror");
                report.warnings.push(msg);
                continue;
            }
            touched.insert(dir_name.clone());

            let mut kept: HashSet<PathBuf> = HashSet::new();
            for asset in batch {
                if let Some(dest) = write_one_asset(&plugin_dir, dir_name, asset, &mut report) {
                    kept.insert(dest);
                }
            }

            // Only a batch carrying the plugin's own manifest describes the whole
            // plugin, so only then can an absent file be read as "upstream
            // dropped it" rather than "this batch did not include it".
            if plan.authoritative.contains(dir_name) {
                sweep_unreferenced(&plugin_dir, dir_name, &kept, &mut report);
                if let Err(e) = fs::write(plugin_dir.join(MIRROR_MARKER), MIRROR_MARKER_BODY) {
                    tracing::warn!(plugin = %dir_name, error = %e, "Could not stamp mirror marker");
                    report.warnings.push(format!(
                        "Could not mark plugins/local/{dir_name} as skrills-managed, so it will \
                         not be pruned later: {e}"
                    ));
                }
            }
        }

        // A batch with no complete plugin in it says nothing about which plugins
        // are still installed, so it must not prune anything.
        if !plan.authoritative.is_empty() {
            prune_stale_mirrors(&local_dir, &touched, &mut report);
        }

        Ok(report)
    }

    fn preview_plugin_assets(&self, assets: &[PluginAsset]) -> Result<WriteReport> {
        let mut report = WriteReport::default();
        if assets.is_empty() {
            return Ok(report);
        }

        let local_dir = self.root.join("plugins").join("local");
        let plan = plan_mirror(&local_dir, assets);
        report.warnings.extend(plan.refusals);
        report.written = plan.accepted.values().map(Vec::len).sum();

        if !plan.authoritative.is_empty() {
            let touched: std::collections::HashSet<String> =
                plan.accepted.keys().cloned().collect();
            let (marked, warnings) = marked_mirror_dirs(&local_dir);
            report.warnings.extend(warnings);
            for name in marked {
                if !touched.contains(&name) {
                    report
                        .warnings
                        .push(format!("Would prune stale plugin mirror: {name}"));
                }
            }
        }

        Ok(report)
    }
}

/// Marker file stamped into every plugin directory this adapter mirrors.
///
/// `plugins/local/` is also where a Cursor user keeps hand-made plugins, so
/// pruning only ever deletes a directory carrying this file. A mirror written by
/// a build that predates the marker is therefore never deleted; it gains one on
/// the next completed mirror of that plugin and is prunable from then on.
pub(crate) const MIRROR_MARKER: &str = ".skrills-mirror";

/// Body of [`MIRROR_MARKER`], so whoever finds the file knows what wrote it.
const MIRROR_MARKER_BODY: &[u8] =
    b"Written by `skrills sync`. Deleting this file stops sync from pruning this directory.\n";

/// What a write batch resolves to.
struct MirrorPlan<'a> {
    /// Destination directory name mapped to the assets going into it.
    accepted: std::collections::BTreeMap<String, Vec<&'a PluginAsset>>,
    /// Directories whose batch carried the plugin's own manifest, so the batch
    /// is the complete content of that plugin.
    authoritative: std::collections::HashSet<String>,
    /// One message per refused plugin.
    refusals: Vec<String>,
}

/// Groups a batch by destination directory and refuses every plugin that cannot
/// be mirrored safely.
///
/// Refusals are per plugin and never an `Err`: plugin assets are the last step
/// of a sync, so returning `Err` here would discard the report for the commands,
/// MCP servers and preferences already written in the same run.
fn plan_mirror<'a>(local_dir: &std::path::Path, assets: &'a [PluginAsset]) -> MirrorPlan<'a> {
    use crate::adapters::utils::sanitize_name;
    use std::collections::{BTreeMap, BTreeSet, HashSet};

    struct Grouped<'a> {
        /// Distinct `publisher/plugin` identities landing in this directory.
        identities: BTreeSet<String>,
        assets: Vec<&'a PluginAsset>,
    }

    let manifest_rel = std::path::Path::new(".claude-plugin").join("plugin.json");

    let mut grouped: BTreeMap<String, Grouped<'a>> = BTreeMap::new();
    for asset in assets {
        let entry = grouped
            .entry(sanitize_name(&asset.plugin_name))
            .or_insert_with(|| Grouped {
                identities: BTreeSet::new(),
                assets: Vec::new(),
            });
        entry
            .identities
            .insert(format!("{}/{}", asset.publisher, asset.plugin_name));
        entry.assets.push(asset);
    }

    let mut accepted = BTreeMap::new();
    let mut authoritative = HashSet::new();
    let mut refusals = Vec::new();

    for (dir_name, group) in grouped {
        let identities = group
            .identities
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");

        if dir_name.is_empty() {
            refusals.push(format!(
                "Skipped plugin {identities}: the name sanitizes to an empty directory name, \
                 which would write into plugins/local itself"
            ));
            continue;
        }

        // Sanitizing strips dots and slashes, so `my.plugin` and `myplugin`, or
        // the same plugin name from two marketplaces, resolve to one directory.
        // Merging their trees makes the on-disk winner flip on every run.
        if group.identities.len() > 1 {
            refusals.push(format!(
                "Skipped {} plugins that all map to plugins/local/{dir_name}: {identities}. \
                 Rename or exclude one of them so each plugin gets its own directory.",
                group.identities.len()
            ));
            continue;
        }

        let carries_manifest = group
            .assets
            .iter()
            .any(|asset| asset.relative_path == manifest_rel);
        if carries_manifest {
            authoritative.insert(dir_name.clone());
        } else if !local_dir
            .join(&dir_name)
            .join(".cursor-plugin")
            .join("plugin.json")
            .exists()
        {
            refusals.push(format!(
                "Skipped plugin {identities}: the batch carries no .claude-plugin/plugin.json and \
                 plugins/local/{dir_name} has no manifest, so Cursor could not load what was \
                 written. Sync with the full plugin mirror."
            ));
            continue;
        }

        accepted.insert(dir_name, group.assets);
    }

    MirrorPlan {
        accepted,
        authoritative,
        refusals,
    }
}

/// Creates a plugin's mirror directory, refusing a symlink or a plain file in
/// its place.
///
/// Called only once the batch is accepted, so a fully refused batch leaves no
/// empty directory behind for the prune loop to protect. A symlink is refused
/// rather than followed: containment is checked against the canonicalized
/// directory, so a pre-planted link would redirect the mirror outside
/// `~/.cursor`.
fn prepare_plugin_dir(plugin_dir: &std::path::Path) -> std::result::Result<(), String> {
    use std::fs;

    match fs::symlink_metadata(plugin_dir) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "Skipped plugin mirror {}: the path is a symlink, which would redirect the write \
             outside ~/.cursor",
            plugin_dir.display()
        )),
        Ok(meta) if !meta.is_dir() => Err(format!(
            "Skipped plugin mirror {}: a file occupies the directory name; remove it to let sync \
             mirror this plugin",
            plugin_dir.display()
        )),
        Ok(_) => Ok(()),
        Err(_) => fs::create_dir_all(plugin_dir).map_err(|e| {
            format!(
                "Could not create plugin mirror {}: {e}",
                plugin_dir.display()
            )
        }),
    }
}

/// Writes one asset and returns its destination when the mirror should keep it.
///
/// Per-asset failures become warnings and the batch continues: a single bad file
/// used to abort `write_plugin_assets` through `?`, which threw away the whole
/// report and failed again on every later sync.
fn write_one_asset(
    plugin_dir: &std::path::Path,
    dir_name: &str,
    asset: &PluginAsset,
    report: &mut WriteReport,
) -> Option<std::path::PathBuf> {
    use crate::adapters::utils::{hash_content, is_path_contained};
    use crate::report::SkipReason;
    use std::fs;
    use std::path::PathBuf;

    let relative = match asset.relative_path.strip_prefix(".claude-plugin") {
        Ok(rest) => PathBuf::from(".cursor-plugin").join(rest),
        Err(_) => asset.relative_path.clone(),
    };
    let dest = plugin_dir.join(&relative);

    if !is_path_contained(&dest, plugin_dir) {
        tracing::warn!(
            plugin = %dir_name,
            path = %asset.relative_path.display(),
            "Plugin asset path escapes plugins/local, skipping"
        );
        report.warnings.push(format!(
            "Skipped plugin asset escaping its plugin directory: {}/{}",
            dir_name,
            asset.relative_path.display()
        ));
        return None;
    }

    if let Err(e) = clear_conflicting_path(plugin_dir, &dest) {
        tracing::warn!(
            plugin = %dir_name,
            path = %relative.display(),
            error = %e,
            "Could not clear the path a plugin asset now needs"
        );
        report.warnings.push(format!(
            "Skipped plugin asset {}/{}: could not clear the conflicting path: {e}",
            dir_name,
            relative.display()
        ));
        return None;
    }

    let existing = if dest.exists() {
        match fs::read(&dest) {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                tracing::debug!(
                    path = %dest.display(),
                    error = %e,
                    "Could not read existing asset for comparison, will re-write"
                );
                None
            }
        }
    } else {
        None
    };
    let content_matches = existing
        .as_deref()
        .is_some_and(|bytes| hash_content(bytes) == hash_content(&asset.content));

    // The content hash says nothing about the mode, so an asset whose bytes are
    // unchanged but whose executable bit moved upstream must not take the
    // unchanged shortcut past the chmod below.
    if content_matches && exec_bit_matches(&dest, asset.executable) {
        report.skipped.push(SkipReason::Unchanged {
            item: format!("{}/{}", dir_name, relative.display()),
        });
        return Some(dest);
    }

    if !content_matches {
        if let Some(parent) = dest.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::warn!(plugin = %dir_name, error = %e, "Could not create asset parent");
                report.warnings.push(format!(
                    "Skipped plugin asset {}/{}: {e}",
                    dir_name,
                    relative.display()
                ));
                return None;
            }
        }
        if let Err(e) = fs::write(&dest, &asset.content) {
            tracing::warn!(plugin = %dir_name, error = %e, "Could not write plugin asset");
            report.warnings.push(format!(
                "Skipped plugin asset {}/{}: {e}",
                dir_name,
                relative.display()
            ));
            return None;
        }
    }

    if let Err(e) = apply_exec_bit(&dest, asset.executable) {
        tracing::warn!(plugin = %dir_name, error = %e, "Could not set plugin asset mode");
        report.warnings.push(format!(
            "Wrote {}/{} but could not set its executable bit: {e}",
            dir_name,
            relative.display()
        ));
    }

    tracing::debug!(
        plugin = %dir_name,
        path = %relative.display(),
        "Wrote plugin asset to Cursor plugins/local"
    );
    report.written += 1;
    Some(dest)
}

/// Removes whatever sits where an asset now has to go.
///
/// A plugin version can turn `scripts/helper` from a file into a directory or
/// the reverse. Without this, `create_dir_all` or `fs::write` fails on that
/// asset on this run and on every run after it.
fn clear_conflicting_path(
    plugin_dir: &std::path::Path,
    dest: &std::path::Path,
) -> std::io::Result<()> {
    use std::fs;

    if let Ok(relative) = dest.strip_prefix(plugin_dir) {
        let components: Vec<_> = relative.components().collect();
        let mut walked = plugin_dir.to_path_buf();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            walked.push(component);
            match fs::symlink_metadata(&walked) {
                Ok(meta) if meta.is_dir() => continue,
                // A file or a symlink: a symlinked parent would also steer the
                // write out of the plugin directory.
                Ok(_) => fs::remove_file(&walked)?,
                // Nothing here yet, so nothing deeper can exist either.
                Err(_) => break,
            }
        }
    }

    match fs::symlink_metadata(dest) {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(dest),
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => fs::remove_file(dest),
        Err(_) => Ok(()),
    }
}

/// Returns true when `dest`'s executable bits already match the asset.
#[cfg(unix)]
fn exec_bit_matches(dest: &std::path::Path, executable: bool) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(dest)
        .map(|meta| (meta.permissions().mode() & 0o111 != 0) == executable)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn exec_bit_matches(_dest: &std::path::Path, _executable: bool) -> bool {
    true
}

/// Sets or clears `dest`'s executable bits to match the asset.
///
/// Only those three bits come from the asset; the rest of the mode is left as
/// the umask made it.
#[cfg(unix)]
fn apply_exec_bit(dest: &std::path::Path, executable: bool) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = std::fs::metadata(dest)?.permissions();
    let current = permissions.mode();
    let desired = if executable {
        current | 0o111
    } else {
        current & !0o111
    };
    if desired != current {
        permissions.set_mode(desired);
        std::fs::set_permissions(dest, permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn apply_exec_bit(_dest: &std::path::Path, _executable: bool) -> std::io::Result<()> {
    Ok(())
}

/// Removes files under `plugin_dir` that this run neither wrote nor skipped.
///
/// The writer only ever added and overwrote, so a plugin upgrade that dropped or
/// renamed `skills/old/SKILL.md` left the old copy in the mirror and Cursor kept
/// loading a skill that no longer exists upstream.
fn sweep_unreferenced(
    plugin_dir: &std::path::Path,
    dir_name: &str,
    kept: &std::collections::HashSet<std::path::PathBuf>,
    report: &mut WriteReport,
) {
    use std::fs;
    use walkdir::WalkDir;

    let marker = plugin_dir.join(MIRROR_MARKER);
    let mut removed = 0usize;

    for entry in WalkDir::new(plugin_dir)
        .min_depth(1)
        .contents_first(true)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(plugin = %dir_name, error = %e, "Could not walk the plugin mirror");
                report.warnings.push(format!(
                    "Could not check plugins/local/{dir_name} for files the plugin no longer \
                     ships: {e}"
                ));
                continue;
            }
        };
        let path = entry.path();
        if path == marker {
            continue;
        }
        if entry.file_type().is_dir() {
            // `contents_first` means the directory is already empty if every file
            // under it went away with the upstream plugin.
            let _ = fs::remove_dir(path);
            continue;
        }
        if kept.contains(path) {
            continue;
        }
        match fs::remove_file(path) {
            Ok(()) => removed += 1,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "Could not remove stale mirror file"
                );
                report.warnings.push(format!(
                    "Failed to remove {} which the plugin no longer ships: {e}",
                    path.display()
                ));
            }
        }
    }

    if removed > 0 {
        tracing::info!(plugin = %dir_name, removed, "Removed files the plugin no longer ships");
        report.warnings.push(format!(
            "Removed {removed} file(s) from plugins/local/{dir_name} that the plugin no longer \
             ships"
        ));
    }
}

/// Lists the `plugins/local` directories carrying [`MIRROR_MARKER`], with one
/// warning per entry that could not be inspected.
fn marked_mirror_dirs(local_dir: &std::path::Path) -> (Vec<String>, Vec<String>) {
    use std::fs;

    let mut marked = Vec::new();
    let mut warnings = Vec::new();

    let entries = match fs::read_dir(local_dir) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::warn!(
                path = %local_dir.display(),
                error = %e,
                "Could not read plugins/local for pruning"
            );
            warnings.push(format!(
                "Could not read {} for pruning: {e}",
                local_dir.display()
            ));
            return (marked, warnings);
        }
    };

    // Per-entry I/O errors used to be dropped with `.filter_map(|e| e.ok())`,
    // which produced a partial prune with no indication. The walk continues past
    // an unreadable entry because one of those should not block unrelated
    // plugins.
    for entry_result in entries {
        let entry = match entry_result {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to read plugins/local entry");
                warnings.push(format!(
                    "Skipped a plugins/local entry due to I/O error: {e} (pruning may be \
                     incomplete)"
                ));
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
                tracing::warn!(?raw, "Skipping non-UTF-8 plugin directory");
                warnings.push(format!(
                    "Skipped non-UTF-8 plugins/local directory name: {raw:?}"
                ));
                continue;
            }
        };
        // An unmarked directory is either a plugin the user installed by hand or
        // a mirror from a build that predates the marker. Never delete it.
        if path.join(MIRROR_MARKER).exists() {
            marked.push(name);
        }
    }

    (marked, warnings)
}

/// Removes the mirrors of plugins that left the batch.
fn prune_stale_mirrors(
    local_dir: &std::path::Path,
    touched: &std::collections::HashSet<String>,
    report: &mut WriteReport,
) {
    use std::fs;

    let (marked, warnings) = marked_mirror_dirs(local_dir);
    report.warnings.extend(warnings);

    for name in marked {
        if touched.contains(&name) {
            continue;
        }
        let path = local_dir.join(&name);
        match fs::remove_dir_all(&path) {
            Ok(()) => {
                tracing::info!(plugin = %name, "Pruned stale local plugin");
                report.warnings.push(format!("Pruned stale plugin: {name}"));
            }
            Err(e) => {
                tracing::warn!(plugin = %name, error = %e, "Failed to prune stale local plugin");
                report
                    .warnings
                    .push(format!("Failed to prune stale plugin {name}: {e}"));
            }
        }
    }
}
