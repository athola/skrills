//! Pre-sync snapshots under `~/.skrills/snapshots/<timestamp>/`, and the
//! rollback that restores them.
//!
//! A snapshot keeps only what a sync touched: a JSON-lines manifest with one
//! [`Entry`] per path, and the prior bytes of each existing file under
//! `blobs/`, named by their SHA-256 so identical files are stored once. Which
//! paths get recorded, and when, is [`crate::write_session`]'s job; this
//! module names, lists, prunes and restores the directories it leaves behind.
//!
//! The directory name is the UTC start time as `YYYYMMDDTHHMMSSZ`, with `-N`
//! appended when two syncs start in the same second. Names give the start
//! time only to the second; [`list`] orders a second's snapshots by `N`, so
//! `-10` counts as newer than `-2`. Retention reads the age from the name
//! rather than the filesystem, so copying or restoring a snapshot directory cannot make it
//! look new. A directory whose name does not parse is not ours and is never
//! listed, rolled back or pruned.

use crate::adapters::utils::{hash_content, remove_dir, remove_file, replace_file_unrecorded};
use crate::write_session::{is_active, WriteSession};
use crate::Result;
use anyhow::{anyhow, bail, Context};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Manifest file name inside a snapshot directory.
pub(crate) const MANIFEST_FILE: &str = "manifest.jsonl";
/// Directory of prior file contents inside a snapshot directory.
pub(crate) const BLOB_DIR: &str = "blobs";

/// Environment variable holding how many days a snapshot is kept.
pub const RETENTION_ENV: &str = "SKRILLS_SNAPSHOT_RETENTION_DAYS";
const DEFAULT_RETENTION_DAYS: u64 = 30;
const SECONDS_PER_DAY: u64 = 86_400;

/// What a path held before the sync touched it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Prior {
    /// Something existed. For [`Kind::File`] its bytes are in
    /// `blobs/<prior_hash>`; a link keeps only its target and a special file
    /// nothing.
    File,
    /// Nothing. Rollback deletes the file the sync created.
    Absent,
}

/// One line of a snapshot manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Absolute path of the file the sync changed.
    pub path: PathBuf,
    /// What the path held before.
    pub prior: Prior,
    /// SHA-256 (lowercase hex) of the prior bytes, also their blob name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_hash: Option<String>,
    /// Unix permission bits of the prior file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    /// What sort of filesystem object the path is. Manifests written before
    /// directories were recorded have no `kind`, and every line in them is a
    /// file.
    #[serde(default, skip_serializing_if = "Kind::is_file")]
    pub kind: Kind,
    /// Where a [`Kind::Symlink`] pointed, exactly as the link held it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<PathBuf>,
    /// What the sync left at the path, from the last after-line for it.
    /// `None` when the manifest has none: the write failed, or recording its
    /// result did. Unless a later snapshot says what skrills left there, such
    /// a path is restored only while it still holds its prior state, and is
    /// otherwise kept as changed since the sync.
    #[serde(skip)]
    pub after: Option<After>,
}

/// What a path held once the sync was done with it. Rollback compares this
/// with the path today, so an edit made after the sync is not overwritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum After {
    /// A regular file with this SHA-256 (lowercase hex).
    File(String),
    /// Nothing: the sync removed the path.
    Absent,
    /// A directory the sync made where it had removed something else.
    Dir,
}

/// A manifest line appended once a recorded change completes. A path may
/// get several; the last one describes the end of the sync.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AfterLine {
    pub(crate) path: PathBuf,
    pub(crate) after: After,
}

/// Either kind of manifest line. An entry always has `prior` and an
/// after-line never does, which is what tells them apart.
#[derive(Deserialize)]
#[serde(untagged)]
enum ManifestLine {
    Entry(Entry),
    After(AfterLine),
}

/// The sort of filesystem object an [`Entry`] describes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A regular file.
    #[default]
    File,
    /// A directory the sync created. Rollback removes it once it is empty.
    Dir,
    /// A symbolic link, kept as its target in [`Entry::link_target`] and
    /// never followed: the file it points to is not the link's own content.
    Symlink,
    /// A fifo, socket or device. Nothing of it can be kept, so rollback
    /// cannot recreate it and says so.
    Special,
}

impl Kind {
    fn is_file(&self) -> bool {
        *self == Kind::File
    }
}

/// A snapshot as `sync-rollback --list` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    /// Directory name under the snapshot root.
    pub name: String,
    /// When the sync that took it started.
    pub taken_at: SystemTime,
    /// Paths recorded in it.
    pub entries: usize,
}

/// What a rollback did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackReport {
    /// The snapshot restored.
    pub snapshot: String,
    /// Files whose prior bytes were written back.
    pub restored: usize,
    /// Files the sync created that were deleted.
    pub removed: usize,
    /// Directories the sync created that were removed.
    pub dirs_removed: usize,
    /// Directories the sync created that were kept because something other
    /// than the sync's own files is in them now, or no longer a directory.
    pub left_in_place: Vec<PathBuf>,
    /// Paths skipped because rollback cannot recreate what was there: a
    /// special file, or a symlink on a platform without Unix links.
    pub not_restorable: Vec<PathBuf>,
    /// Paths that hold neither what the sync left, nor what a later sync
    /// left, nor what was there before, so someone changed them since; they
    /// are kept as they are. [`force_rollback`] restores them too.
    pub changed_since_sync: Vec<PathBuf>,
    /// Paths rollback tried to restore and could not, each with the reason.
    /// Everything else in the report was done anyway.
    pub failed: Vec<(PathBuf, String)>,
}

/// `~/.skrills/snapshots`, or `None` when no home directory is known.
///
/// Home is resolved as [`crate::kill_switch_marker::marker_path`] does it: an
/// absolute `HOME` first, then the platform lookup.
pub fn default_root() -> Option<PathBuf> {
    skrills_home().map(|home| home.join("snapshots"))
}

/// `~/.skrills`, where snapshots and sync state both live.
pub(crate) fn skrills_home() -> Option<PathBuf> {
    #[cfg(unix)]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if home.is_absolute() {
            return Some(home.join(".skrills"));
        }
    }
    dirs::home_dir()
        .filter(|home| home.is_absolute())
        .map(|home| home.join(".skrills"))
}

/// How long a snapshot is kept: [`RETENTION_ENV`] days, 30 when unset.
pub fn retention() -> Duration {
    retention_from(std::env::var(RETENTION_ENV).ok().as_deref())
}

fn retention_from(setting: Option<&str>) -> Duration {
    let days = match setting.map(str::trim) {
        None | Some("") => DEFAULT_RETENTION_DAYS,
        Some(text) => text.parse().unwrap_or_else(|_| {
            tracing::warn!(
                value = text,
                "{RETENTION_ENV} is not a whole number of days; keeping snapshots {DEFAULT_RETENTION_DAYS} days"
            );
            DEFAULT_RETENTION_DAYS
        }),
    };
    Duration::from_secs(days.saturating_mul(SECONDS_PER_DAY))
}

/// Prunes expired snapshots under `root`, then starts recording into a new
/// one.
///
/// # Panics
///
/// Panics when a session is already installed on this thread; see
/// [`WriteSession::install`].
pub fn begin(root: &Path) -> std::io::Result<WriteSession> {
    begin_at(root, SystemTime::now())
}

fn begin_at(root: &Path, now: SystemTime) -> std::io::Result<WriteSession> {
    if !root.is_dir() {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(root)?;
    }
    if let Err(e) = prune(root, now, retention()) {
        tracing::warn!(path = %root.display(), error = %e, "Could not prune old snapshots");
    }
    let stem = format_name(now);
    let mut attempt = 0_u32;
    loop {
        let name = match attempt {
            0 => stem.clone(),
            n => format!("{stem}-{n}"),
        };
        match WriteSession::install(&root.join(name)) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => attempt += 1,
            other => return other,
        }
    }
}

/// [`begin`] at [`default_root`], or `None` after a warning when that is not
/// possible. A snapshot is a safety net, so failing to take one does not stop
/// the sync it was meant to protect.
pub fn begin_default() -> Option<WriteSession> {
    let Some(root) = default_root() else {
        tracing::warn!("No home directory; this sync will not be snapshotted");
        return None;
    };
    begin_or_warn(&root)
}

/// Removes the directory tree at `path`, recording every file in it into the
/// installed session first so [`rollback`] can bring the tree back.
///
/// For commands outside this crate that delete user files (`skrills prune`):
/// the sync crate's own write helpers are crate-private, and a raw
/// `std::fs::remove_dir_all` would leave nothing to roll back. Outside a
/// session it only removes the tree. Only files and links are recorded, so
/// rollback brings back each directory that held one, but not an empty
/// subdirectory.
pub fn remove_recorded_tree(path: &Path) -> std::io::Result<()> {
    crate::adapters::utils::remove_tree(path)
}

pub(crate) fn begin_or_warn(root: &Path) -> Option<WriteSession> {
    match begin(root) {
        Ok(session) => Some(session),
        Err(e) => {
            tracing::warn!(
                path = %root.display(),
                error = %e,
                "Could not start a snapshot; this sync will not be snapshotted"
            );
            None
        }
    }
}

/// Snapshots under `root`, newest first. A missing `root` holds none.
pub fn list(root: &Path) -> Result<Vec<SnapshotInfo>> {
    let reader = match std::fs::read_dir(root) {
        Ok(reader) => reader,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("Failed to read {}", root.display())),
    };
    let mut snapshots = Vec::new();
    for entry in reader {
        let entry = entry.with_context(|| format!("Failed to read {}", root.display()))?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(taken_at) = parse_name(&name) else {
            continue;
        };
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let entries = read_manifest(&entry.path()).map_or_else(
            |e| {
                tracing::warn!(snapshot = %name, error = %e, "Unreadable snapshot manifest");
                0
            },
            |entries| entries.len(),
        );
        snapshots.push(SnapshotInfo {
            name,
            taken_at,
            entries,
        });
    }
    snapshots.sort_by_key(|snapshot| {
        std::cmp::Reverse((snapshot.taken_at, same_second_index(&snapshot.name)))
    });
    Ok(snapshots)
}

/// The `N` of a `-N` name suffix, 0 without one, so `-10` counts as newer than `-2`.
fn same_second_index(name: &str) -> u64 {
    name.get(16..)
        .and_then(|suffix| suffix.strip_prefix('-'))
        .map_or(0, |n| n.parse().unwrap_or(u64::MAX))
}

/// Deletes snapshots under `root` taken more than `keep` before `now`, and
/// returns how many went. One whose sync is still running is kept, whatever
/// its age; one that cannot be deleted is logged and left.
pub fn prune(root: &Path, now: SystemTime, keep: Duration) -> Result<usize> {
    let mut pruned = 0;
    for snapshot in list(root)? {
        let age = now.duration_since(snapshot.taken_at).unwrap_or_default();
        if age <= keep {
            continue;
        }
        let dir = root.join(&snapshot.name);
        match try_claim(&dir, false) {
            // Released before the delete: Windows cannot remove an open file.
            Ok(Claim::Free(lock)) => drop(lock),
            Ok(Claim::InUse) => continue,
            Err(e) => {
                tracing::warn!(snapshot = %snapshot.name, error = %e, "Could not check an expired snapshot");
                continue;
            }
        }
        match crate::adapters::utils::remove_tree_unrecorded(&dir) {
            Ok(()) => pruned += 1,
            Err(e) => tracing::warn!(
                snapshot = %snapshot.name,
                error = %e,
                "Could not delete an expired snapshot"
            ),
        }
    }
    Ok(pruned)
}

/// Restores `name`, or the newest snapshot whose sync has finished when
/// `name` is `None`. A named snapshot whose sync is still running is
/// refused.
///
/// The manifest and saved contents are checked before the first file
/// changes: a path outside what a sync could have written (relative, or
/// containing `..`), an inconsistent entry, or saved content that is missing
/// or no longer matches its hash refuses the whole rollback instead of half
/// of it. A failure the check cannot foresee (an I/O error, or a directory
/// the user filled where a file was) stops only that entry: it is listed in
/// [`RollbackReport::failed`] and the rest is still restored.
///
/// Entries are undone newest first, so a path a sync replaced with a
/// directory, or the reverse, comes back in the order it was lost. The
/// directories a sync created under such a path are removed with it; the
/// other directories it created go last, deepest first, once the files in
/// them are gone, and one that still holds anything is left and reported.
///
/// A path that holds neither what the sync left there nor what it held
/// before was changed since, and is kept and listed in
/// [`RollbackReport::changed_since_sync`]. When an older snapshot is named,
/// a path a newer sync rewrote is checked against what the newest such sync
/// left as well, so skrills' own later changes are restored and only someone
/// else's are kept.
pub fn rollback(root: &Path, name: Option<&str>) -> Result<RollbackReport> {
    rollback_checked(root, name, false)
}

/// [`rollback`] that also restores paths changed since the sync, discarding
/// those changes.
pub fn force_rollback(root: &Path, name: Option<&str>) -> Result<RollbackReport> {
    rollback_checked(root, name, true)
}

fn rollback_checked(root: &Path, name: Option<&str>, force: bool) -> Result<RollbackReport> {
    let mut snapshots = list(root)?;
    // Held until the rollback is done, so pruning in another process leaves
    // this snapshot alone meanwhile.
    let (position, _claim) = claim_snapshot(root, &snapshots, name)?;
    let left_by_later_syncs = later_after_states(root, &snapshots[..position]);
    let chosen = snapshots.swap_remove(position);
    let dir = root.join(&chosen.name);
    let entries = read_manifest(&dir)?;
    let blobs = entries
        .iter()
        .map(|entry| checked_blob(&dir, entry))
        .collect::<Result<Vec<_>>>()?;

    let mut report = RollbackReport {
        snapshot: chosen.name,
        restored: 0,
        removed: 0,
        dirs_removed: 0,
        left_in_place: Vec::new(),
        not_restorable: Vec::new(),
        changed_since_sync: Vec::new(),
        failed: Vec::new(),
    };
    let mut created_dirs = Vec::new();
    for (entry, blob) in entries.iter().zip(&blobs).rev() {
        match entry.kind {
            Kind::Dir => {
                created_dirs.push(entry.path.as_path());
                continue;
            }
            Kind::Special => {
                tracing::warn!(path = %entry.path.display(), "Cannot recreate a special file");
                report.not_restorable.push(entry.path.clone());
                continue;
            }
            Kind::File | Kind::Symlink => {}
        }
        let left_by_skrills: Vec<&After> =
            [entry.after.as_ref(), left_by_later_syncs.get(&entry.path)]
                .into_iter()
                .flatten()
                .collect();
        let undone = undo_entry(
            entry,
            blob.as_deref(),
            &left_by_skrills,
            force,
            &mut created_dirs,
            &mut report,
        );
        if let Err(e) = undone {
            report.failed.push((entry.path.clone(), format!("{e:#}")));
        }
    }
    created_dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for dir in created_dirs {
        match remove_created_dir(dir) {
            Ok(true) => report.dirs_removed += 1,
            Ok(false) if dir.symlink_metadata().is_ok() => {
                report.left_in_place.push(dir.to_path_buf());
            }
            Ok(false) => {}
            Err(e) => report.failed.push((dir.to_path_buf(), format!("{e:#}"))),
        }
    }
    Ok(report)
}

/// Whether a snapshot's sync is still running, as [`try_claim`] found it.
enum Claim {
    /// Nobody is writing it. The lock is `None` when there is no manifest
    /// to lock or the filesystem has no locks.
    Free(Option<std::fs::File>),
    /// A running sync still holds its manifest, or (for an exclusive claim)
    /// a rollback is reading it.
    InUse,
}

/// Locks the manifest in `dir`, which [`WriteSession`] holds exclusively for
/// its whole life. The OS releases that lock when the process ends, so a
/// sync that was killed leaves a snapshot that is free at once, and
/// snapshots written before the lock existed need no marker.
///
/// A rollback claims `shared`, because Windows locks are mandatory and an
/// exclusive lock would stop its own reads of the manifest; pruning claims
/// exclusively, so it also leaves alone a snapshot being rolled back.
fn try_claim(dir: &Path, shared: bool) -> Result<Claim> {
    let path = dir.join(MANIFEST_FILE);
    let manifest = match std::fs::File::open(&path) {
        Ok(manifest) => manifest,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Claim::Free(None)),
        Err(e) => return Err(e).with_context(|| format!("Failed to open {}", path.display())),
    };
    let locked = if shared {
        manifest.try_lock_shared()
    } else {
        manifest.try_lock()
    };
    match locked {
        Ok(()) => Ok(Claim::Free(Some(manifest))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(Claim::InUse),
        Err(std::fs::TryLockError::Error(e)) if e.kind() == std::io::ErrorKind::Unsupported => {
            Ok(Claim::Free(None))
        }
        Err(std::fs::TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("Failed to lock {}", path.display()))
        }
    }
}

/// The index in `snapshots` of the one to roll back, and its lock: `name`,
/// or the newest whose sync has finished.
fn claim_snapshot(
    root: &Path,
    snapshots: &[SnapshotInfo],
    name: Option<&str>,
) -> Result<(usize, Option<std::fs::File>)> {
    if let Some(wanted) = name {
        let position = snapshots
            .iter()
            .position(|snapshot| snapshot.name == wanted)
            .ok_or_else(|| anyhow!("no snapshot named {wanted} in {}", root.display()))?;
        return match try_claim(&root.join(wanted), true)? {
            Claim::Free(lock) => Ok((position, lock)),
            Claim::InUse => bail!(
                "snapshot {wanted} belongs to a sync that is still running; roll it back once that sync ends"
            ),
        };
    }
    if snapshots.is_empty() {
        bail!("no snapshots in {}", root.display());
    }
    for (position, snapshot) in snapshots.iter().enumerate() {
        if let Claim::Free(lock) = try_claim(&root.join(&snapshot.name), true)? {
            return Ok((position, lock));
        }
    }
    bail!(
        "every snapshot in {} belongs to a sync that is still running",
        root.display()
    )
}

/// Puts one file or link entry back, or lists it as changed since the sync.
/// Counts go into `report`; an error is this entry's alone.
fn undo_entry(
    entry: &Entry,
    blob: Option<&Path>,
    left_by_skrills: &[&After],
    force: bool,
    created_dirs: &mut Vec<&Path>,
    report: &mut RollbackReport,
) -> Result<()> {
    if !force && !holds_what_skrills_left(entry, left_by_skrills)? {
        report.changed_since_sync.push(entry.path.clone());
        return Ok(());
    }
    if entry.kind == Kind::Symlink {
        #[cfg(unix)]
        {
            report.dirs_removed += restore_link(entry, created_dirs)?;
            report.restored += 1;
        }
        #[cfg(not(unix))]
        report.not_restorable.push(entry.path.clone());
        return Ok(());
    }
    match blob {
        Some(blob) => {
            report.dirs_removed += restore_file(entry, blob, created_dirs)?;
            report.restored += 1;
        }
        None => {
            if delete_created(&entry.path)? {
                report.removed += 1;
            }
        }
    }
    Ok(())
}

/// Whether `entry`'s path holds what a sync of skrills left there
/// (`left_by_skrills`), or still holds what it held before the sync. With no
/// after-state on record (recording it failed), only the prior state counts,
/// so an edit made since is kept rather than overwritten unseen.
fn holds_what_skrills_left(entry: &Entry, left_by_skrills: &[&After]) -> Result<bool> {
    if entry.kind == Kind::Symlink {
        if let (Ok(target), Some(prior)) = (
            std::fs::read_link(&entry.path),
            entry.link_target.as_deref(),
        ) {
            if target == prior {
                return Ok(true);
            }
        }
    }
    let Some(now) = current_state(&entry.path)? else {
        return Ok(false);
    };
    let prior = match (entry.prior, entry.kind) {
        (Prior::Absent, _) => Some(After::Absent),
        (Prior::File, Kind::File) => entry.prior_hash.clone().map(After::File),
        (Prior::File, _) => None,
    };
    Ok(prior.as_ref() == Some(&now) || left_by_skrills.contains(&&now))
}

/// The newest after-state per path across `newer`, the snapshots taken
/// after the one being rolled back (newest first, as [`list`] gives them).
///
/// A later sync that rewrote a path left it in that state, so finding it
/// there means skrills made the change, not the user. A manifest that
/// cannot be read is skipped with a warning: its paths then fall back to
/// the rolled-back snapshot's own after-state, which keeps more, not less.
fn later_after_states(
    root: &Path,
    newer: &[SnapshotInfo],
) -> std::collections::HashMap<PathBuf, After> {
    let mut states = std::collections::HashMap::new();
    for snapshot in newer.iter().rev() {
        match read_manifest(&root.join(&snapshot.name)) {
            Ok(entries) => states.extend(
                entries
                    .into_iter()
                    .filter_map(|entry| Some((entry.path, entry.after?))),
            ),
            Err(e) => tracing::warn!(
                snapshot = %snapshot.name,
                error = %e,
                "Ignoring an unreadable newer snapshot while checking for later changes"
            ),
        }
    }
    states
}

/// What `path` holds now, in the terms of an after-line; `None` for
/// something no after-line describes (a link or special file).
/// The type is checked before any read, so a fifo is never opened.
fn current_state(path: &Path) -> Result<Option<After>> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => Ok(Some(After::Dir)),
        Ok(meta) if meta.is_file() => {
            let bytes = std::fs::read(path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            Ok(Some(After::File(hash_content(&bytes))))
        }
        Ok(_) => Ok(None),
        Err(e) if is_gone(&e) => Ok(Some(After::Absent)),
        Err(e) => Err(e).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

/// Whether `error` from inspecting a path means nothing is there: it is
/// missing, or one of its parents is no longer a directory.
fn is_gone(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

/// Removes a directory the sync created if it is still an empty directory.
/// Returns whether it was removed; something else at the path, or a
/// directory the user has put files in since, is not an error.
fn remove_created_dir(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => match remove_dir(path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => Ok(false),
            Err(e) => Err(e).with_context(|| format!("Failed to remove {}", path.display())),
        },
        Ok(_) => Ok(false),
        Err(e) if is_gone(&e) => Ok(false),
        Err(e) => Err(e).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

/// Reads a snapshot's manifest. A final line without its newline is a write
/// an interrupt cut short, so it is skipped; any other bad line is an error.
/// After-lines are folded into the entry for their path, the last one
/// winning; one for a path with no entry is ignored.
pub(crate) fn read_manifest(dir: &Path) -> Result<Vec<Entry>> {
    let path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let mut entries: Vec<Entry> = Vec::with_capacity(lines.len());
    let mut by_path = std::collections::HashMap::new();
    for (index, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(ManifestLine::Entry(entry)) => {
                by_path.insert(entry.path.clone(), entries.len());
                entries.push(entry);
            }
            Ok(ManifestLine::After(after_line)) => match by_path.get(&after_line.path) {
                Some(&position) => entries[position].after = Some(after_line.after),
                None => tracing::warn!(
                    path = %after_line.path.display(),
                    "Ignoring an after-line for a path the snapshot never recorded"
                ),
            },
            Err(_) if index + 1 == lines.len() && !text.ends_with('\n') => {
                tracing::warn!(path = %path.display(), "Ignoring a manifest line cut short by an interrupt");
            }
            Err(e) => bail!("{}: line {}: {e}", path.display(), index + 1),
        }
    }
    Ok(entries)
}

/// Validates `entry` and returns the blob holding its prior bytes, or `None`
/// for a path that did not exist.
fn checked_blob(dir: &Path, entry: &Entry) -> Result<Option<PathBuf>> {
    if !entry.path.is_absolute() {
        bail!(
            "refusing manifest path {}: not absolute",
            entry.path.display()
        );
    }
    if entry
        .path
        .components()
        .any(|part| part == Component::ParentDir)
    {
        bail!(
            "refusing manifest path {}: contains `..`",
            entry.path.display()
        );
    }
    if entry.prior == Prior::Absent {
        if matches!(entry.kind, Kind::Symlink | Kind::Special) {
            bail!(
                "manifest entry for {} is a link or special file recorded as absent",
                entry.path.display()
            );
        }
        return Ok(None);
    }
    match entry.kind {
        Kind::Symlink if entry.link_target.is_none() => bail!(
            "manifest entry for {} is a symlink with no link target",
            entry.path.display()
        ),
        Kind::Symlink | Kind::Special | Kind::Dir => return Ok(None),
        Kind::File => {}
    }
    let hash = entry
        .prior_hash
        .as_deref()
        .filter(|hash| {
            hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
        .ok_or_else(|| {
            anyhow!(
                "manifest entry for {} has no valid prior hash",
                entry.path.display()
            )
        })?;
    let blob = dir.join(BLOB_DIR).join(hash);
    if !blob.is_file() {
        bail!(
            "snapshot is missing the saved content of {}",
            entry.path.display()
        );
    }
    // Checked here as well as on restore, so a corrupt blob refuses the
    // rollback before anything changes rather than halfway through it.
    let bytes =
        std::fs::read(&blob).with_context(|| format!("Failed to read {}", blob.display()))?;
    if hash_content(&bytes) != hash {
        bail!("saved content of {} is corrupt", entry.path.display());
    }
    Ok(Some(blob))
}

/// Writes the prior bytes and mode back to the path itself, replacing a
/// symlink there rather than writing through it. Returns how many created
/// directories it removed to make room.
fn restore_file(entry: &Entry, blob: &Path, created_dirs: &mut Vec<&Path>) -> Result<usize> {
    let bytes =
        std::fs::read(blob).with_context(|| format!("Failed to read {}", blob.display()))?;
    if Some(hash_content(&bytes).as_str()) != entry.prior_hash.as_deref() {
        bail!("saved content of {} is corrupt", entry.path.display());
    }
    let dirs_removed = clear_directory_in_the_way(&entry.path, created_dirs)?;
    replace_file_unrecorded(&entry.path, &bytes, entry.mode)
        .with_context(|| format!("Failed to restore {}", entry.path.display()))?;
    Ok(dirs_removed)
}

/// Removes a directory a sync put where `path` held a file or link, so the
/// file or link can come back. The sync's own files in it are gone by now,
/// so the directories it created under `path` are removed first, deepest
/// first, and taken off `created_dirs`; what is left must be empty, or it
/// is the user's and the restore fails. Returns how many directories went.
fn clear_directory_in_the_way(path: &Path, created_dirs: &mut Vec<&Path>) -> Result<usize> {
    if !std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
        return Ok(0);
    }
    let mut below: Vec<&Path> = created_dirs
        .iter()
        .copied()
        .filter(|dir| dir.starts_with(path) && *dir != path)
        .collect();
    created_dirs.retain(|dir| !below.contains(dir));
    below.sort_by_key(|dir| std::cmp::Reverse(dir.components().count()));
    let mut removed = 0;
    for dir in below {
        if remove_created_dir(dir)? {
            removed += 1;
        }
    }
    remove_dir(path)
        .with_context(|| format!("{} is now a directory that is not empty", path.display()))?;
    Ok(removed)
}

/// Puts back a symlink the sync removed or replaced, pointing exactly where it
/// pointed before. Returns how many created directories it removed to make
/// room.
#[cfg(unix)]
fn restore_link(entry: &Entry, created_dirs: &mut Vec<&Path>) -> Result<usize> {
    let target = entry.link_target.as_deref().ok_or_else(|| {
        anyhow!(
            "manifest entry for {} is a symlink with no link target",
            entry.path.display()
        )
    })?;
    let dirs_removed = clear_directory_in_the_way(&entry.path, created_dirs)?;
    crate::adapters::utils::replace_with_symlink(&entry.path, target)
        .with_context(|| format!("Failed to restore the link {}", entry.path.display()))?;
    Ok(dirs_removed)
}

/// Deletes a file the sync created. Returns whether there was one to delete.
fn delete_created(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => {
            tracing::warn!(path = %path.display(), "Leaving a directory where the sync had created a file");
            Ok(false)
        }
        Ok(_) => {
            remove_file(path).with_context(|| format!("Failed to delete {}", path.display()))?;
            Ok(true)
        }
        Err(e) if is_gone(&e) => Ok(false),
        Err(e) => Err(e).with_context(|| format!("Failed to inspect {}", path.display())),
    }
}

/// `20261113T142530Z`: the UTC second `time` falls in.
fn format_name(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(time);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// `2026-11-13 14:25:30 UTC`, for `--list`.
pub fn display_time(time: SystemTime) -> String {
    let (year, month, day, hour, minute, second) = civil(time);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

pub(crate) fn civil(time: SystemTime) -> (i64, i64, i64, i64, i64, i64) {
    let seconds = time.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
    });
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Howard Hinnant's days-to-civil conversion.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day, rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The start time a snapshot directory name encodes, if it is one.
fn parse_name(name: &str) -> Option<SystemTime> {
    let (stamp, suffix) = name.split_at_checked(16)?;
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = stamp.get(range)?;
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let suffix_ok = suffix.is_empty()
        || suffix
            .strip_prefix('-')
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    if !suffix_ok || stamp.as_bytes()[8] != b'T' || stamp.as_bytes()[15] != b'Z' {
        return None;
    }
    let (year, month, day) = (digits(0..4)?, digits(4..6)?, digits(6..8)?);
    let (hour, minute, second) = (digits(9..11)?, digits(11..13)?, digits(13..15)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let shifted_year = year - i64::from(month <= 2);
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year.rem_euclid(400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = u64::try_from(days * 86_400 + hour * 3600 + minute * 60 + second).ok()?;
    Some(UNIX_EPOCH + Duration::from_secs(seconds))
}

/// Joins an already-active session instead of starting one; see
/// [`crate::orchestrator::SyncOrchestrator::sync`].
pub(crate) fn begin_unless_active(root: &Path) -> Option<WriteSession> {
    if is_active() {
        None
    } else {
        begin_or_warn(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::utils::{remove_tree, write_file};
    use skrills_test_utils::{env_guard, set_env_var};

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    /// 2026-11-13 14:25:30 UTC.
    const SAMPLE: u64 = 1_794_579_930;

    fn drop_a_recorded_change(root: &Path, now: SystemTime) -> PathBuf {
        let target = root.join("work").join("target.txt");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"before").unwrap();
        let session = begin_at(&root.join("snaps"), now).unwrap();
        write_file(&target, b"after").unwrap();
        drop(session);
        target
    }

    #[test]
    fn remove_recorded_tree_is_undone_by_rollback() {
        let tmp = tempfile::tempdir().unwrap();
        let tree = tmp.path().join("skills").join("old-skill");
        std::fs::create_dir_all(tree.join("references")).unwrap();
        std::fs::write(tree.join("SKILL.md"), b"body").unwrap();
        std::fs::write(tree.join("references/notes.md"), b"notes").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();

        remove_recorded_tree(&tree).unwrap();
        drop(session);
        assert!(!tree.exists());
        rollback(&root, None).unwrap();

        assert_eq!(std::fs::read(tree.join("SKILL.md")).unwrap(), b"body");
        assert_eq!(
            std::fs::read(tree.join("references/notes.md")).unwrap(),
            b"notes"
        );
    }

    #[test]
    fn name_round_trips_through_format_and_parse() {
        assert_eq!(format_name(at(SAMPLE)), "20261113T142530Z");
        assert_eq!(parse_name("20261113T142530Z"), Some(at(SAMPLE)));
        assert_eq!(parse_name("20261113T142530Z-2"), Some(at(SAMPLE)));
    }

    #[test]
    fn leap_day_is_formatted_and_parsed() {
        let leap = 1_709_210_096; // 2024-02-29 12:34:56 UTC
        assert_eq!(format_name(at(leap)), "20240229T123456Z");
        assert_eq!(parse_name("20240229T123456Z"), Some(at(leap)));
    }

    #[test]
    fn display_time_is_human_readable_utc() {
        assert_eq!(display_time(at(SAMPLE)), "2026-11-13 14:25:30 UTC");
    }

    #[test]
    fn parse_rejects_names_that_are_not_snapshots() {
        for name in [
            "",
            "latest",
            "20261113T142530",
            "20261113 142530Z",
            "20261113T142530Zjunk",
            "20261113T142530Z-",
            "20261113T142530Z-x",
            "2026111xT142530Z",
            "20261313T142530Z",
            "20261100T142530Z",
            "20261113T242530Z",
            "20261113T146030Z",
            "20261113T142560Z",
        ] {
            assert_eq!(parse_name(name), None, "{name:?} should not parse");
        }
    }

    #[test]
    fn parse_rejects_signs_that_integer_parsing_would_accept() {
        assert_eq!(parse_name("+0261113T142530Z"), None);
    }

    #[test]
    fn retention_defaults_to_thirty_days() {
        assert_eq!(retention_from(None), Duration::from_secs(30 * 86_400));
        assert_eq!(retention_from(Some("  ")), Duration::from_secs(30 * 86_400));
    }

    #[test]
    fn retention_reads_a_number_of_days() {
        assert_eq!(retention_from(Some("7")), Duration::from_secs(7 * 86_400));
        assert_eq!(retention_from(Some("0")), Duration::ZERO);
    }

    #[test]
    fn retention_falls_back_to_the_default_on_garbage() {
        assert_eq!(
            retention_from(Some("a week")),
            Duration::from_secs(30 * 86_400)
        );
        assert_eq!(retention_from(Some("-3")), Duration::from_secs(30 * 86_400));
    }

    #[test]
    fn retention_reads_the_environment_variable() {
        let _lock = env_guard();
        let _env = set_env_var(RETENTION_ENV, Some("3"));
        assert_eq!(retention(), Duration::from_secs(3 * 86_400));
    }

    #[test]
    fn default_root_lives_under_home() {
        let _lock = env_guard();
        let home = tempfile::tempdir().unwrap();
        let _env = set_env_var("HOME", Some(home.path().to_str().unwrap()));
        assert_eq!(
            default_root(),
            Some(home.path().join(".skrills").join("snapshots"))
        );
    }

    #[test]
    fn begin_default_snapshots_under_the_homes_skrills_directory() {
        let _lock = env_guard();
        let home = tempfile::tempdir().unwrap();
        let _env = set_env_var("HOME", Some(home.path().to_str().unwrap()));
        let target = home.path().join("cfg.json");
        std::fs::write(&target, b"{}").unwrap();
        let session = begin_default().expect("a session under the temporary home");
        write_file(&target, b"{\"a\":1}").unwrap();
        drop(session);
        let root = home.path().join(".skrills").join("snapshots");
        assert_eq!(list(&root).unwrap().len(), 1);
    }

    #[test]
    fn begin_creates_a_private_root_and_a_named_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("deep").join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        assert!(root.join("20261113T142530Z").is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&root).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
        }
        drop(session);
    }

    #[test]
    fn begin_in_the_same_second_picks_a_distinct_name() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        std::fs::create_dir_all(root.join("20261113T142530Z")).unwrap();
        std::fs::create_dir_all(root.join("20261113T142530Z-1")).unwrap();
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        assert!(root.join("20261113T142530Z-2").is_dir());
        drop(session);
    }

    #[test]
    fn begin_reports_a_root_it_cannot_create() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        std::fs::write(&file, b"x").unwrap();
        assert!(begin(&file.join("snaps")).is_err());
        assert!(begin_or_warn(&file.join("snaps")).is_none());
        assert!(!is_active());
    }

    #[test]
    fn begin_unless_active_joins_a_running_session() {
        let tmp = tempfile::tempdir().unwrap();
        let outer = begin_at(&tmp.path().join("snaps"), at(SAMPLE)).unwrap();
        assert!(begin_unless_active(&tmp.path().join("other")).is_none());
        assert!(!tmp.path().join("other").exists());
        drop(outer);
        assert!(begin_unless_active(&tmp.path().join("other")).is_some());
    }

    #[test]
    fn begin_prunes_snapshots_past_retention_before_starting() {
        let _lock = env_guard();
        let _env = set_env_var(RETENTION_ENV, Some("30"));
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        std::fs::create_dir_all(root.join("20200101T000000Z")).unwrap();
        let session = begin(&root).unwrap();
        assert!(!root.join("20200101T000000Z").exists());
        drop(session);
    }

    #[test]
    fn a_sync_that_changed_nothing_leaves_no_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        drop(begin_at(&root, at(SAMPLE)).unwrap());
        assert!(list(&root).unwrap().is_empty());
    }

    #[test]
    fn list_of_a_missing_root_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(list(&tmp.path().join("nope")).unwrap().is_empty());
    }

    #[test]
    fn list_of_a_root_that_is_a_file_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert!(list(&file).is_err());
    }

    #[test]
    fn list_is_newest_first_with_entry_counts_and_skips_strangers() {
        let tmp = tempfile::tempdir().unwrap();
        drop_a_recorded_change(tmp.path(), at(SAMPLE));
        let root = tmp.path().join("snaps");
        std::fs::create_dir_all(root.join("20200101T000000Z")).unwrap();
        std::fs::write(root.join("20200101T000000Z").join(MANIFEST_FILE), b"").unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();
        std::fs::write(root.join("20990101T000000Z"), b"a file, not a snapshot").unwrap();

        let snapshots = list(&root).unwrap();
        let names: Vec<_> = snapshots.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["20261113T142530Z", "20200101T000000Z"]);
        assert_eq!(snapshots[0].entries, 1);
        assert_eq!(snapshots[0].taken_at, at(SAMPLE));
        assert_eq!(snapshots[1].entries, 0);
    }

    #[test]
    fn list_counts_a_snapshot_with_an_unreadable_manifest_as_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        std::fs::create_dir_all(root.join("20200101T000000Z")).unwrap();
        let snapshots = list(&root).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].entries, 0);
    }

    #[test]
    fn prune_removes_only_snapshots_older_than_retention() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        for name in [
            "20261001T000000Z", // 43 days before SAMPLE
            "20261020T000000Z", // 24 days before SAMPLE
            "20261113T142530Z",
        ] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        std::fs::create_dir_all(root.join("keep-me")).unwrap();

        let pruned = prune(&root, at(SAMPLE), Duration::from_secs(30 * 86_400)).unwrap();
        assert_eq!(pruned, 1);
        assert!(!root.join("20261001T000000Z").exists());
        assert!(root.join("20261020T000000Z").exists());
        assert!(root.join("20261113T142530Z").exists());
        assert!(
            root.join("keep-me").exists(),
            "a directory that is not a snapshot is left alone"
        );
    }

    #[test]
    fn prune_keeps_a_snapshot_dated_in_the_future() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        std::fs::create_dir_all(root.join("20990101T000000Z")).unwrap();
        assert_eq!(prune(&root, at(SAMPLE), Duration::ZERO).unwrap(), 0);
        assert!(root.join("20990101T000000Z").exists());
    }

    #[test]
    fn prune_of_a_missing_root_removes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(
            prune(&tmp.path().join("nope"), at(SAMPLE), Duration::ZERO).unwrap(),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn prune_logs_and_continues_when_a_snapshot_cannot_be_deleted() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let stuck = root.join("20200101T000000Z");
        std::fs::create_dir_all(stuck.join("inner")).unwrap();
        let readable = root.join("20200102T000000Z");
        std::fs::create_dir_all(&readable).unwrap();
        std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o500)).unwrap();

        let pruned = prune(&root, at(SAMPLE), Duration::ZERO).unwrap();
        let stuck_survived = stuck.exists();
        std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o700)).unwrap();

        // A privileged user can delete the locked directory; anyone else must
        // see it skipped while the other expired snapshot still goes.
        assert!(!readable.exists());
        assert_eq!(pruned, if stuck_survived { 1 } else { 2 });
    }

    #[test]
    fn rollback_restores_prior_bytes_of_a_file_a_sync_overwrote() {
        let tmp = tempfile::tempdir().unwrap();
        let target = drop_a_recorded_change(tmp.path(), at(SAMPLE));
        assert_eq!(std::fs::read(&target).unwrap(), b"after");

        let report = rollback(&tmp.path().join("snaps"), None).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"before");
        assert_eq!(report.snapshot, "20261113T142530Z");
        assert_eq!((report.restored, report.removed), (1, 0));
    }

    #[cfg(unix)]
    #[test]
    fn rollback_restores_the_prior_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("run.sh");
        std::fs::write(&target, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o750)).unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&target, b"changed").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(session);

        rollback(&root, None).unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750);
    }

    #[test]
    fn rollback_deletes_a_file_the_sync_created() {
        let tmp = tempfile::tempdir().unwrap();
        let created = tmp.path().join("new").join("made-by-sync.md");
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&created, b"fresh").unwrap();
        drop(session);
        assert!(created.exists());

        let report = rollback(&root, None).unwrap();
        assert!(!created.exists());
        assert_eq!((report.restored, report.removed), (0, 1));
    }

    /// Writes `new/deeper/made.md` below `base` in a recorded sync, so the
    /// sync creates two directories.
    fn sync_creating_directories(base: &Path, root: &Path) -> PathBuf {
        let made = base.join("new").join("deeper").join("made.md");
        let session = begin_at(root, at(SAMPLE)).unwrap();
        write_file(&made, b"fresh").unwrap();
        drop(session);
        made
    }

    #[test]
    fn rollback_removes_the_directories_the_sync_created() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        sync_creating_directories(tmp.path(), &root);

        let report = rollback(&root, None).unwrap();
        assert!(!tmp.path().join("new").exists());
        assert_eq!((report.removed, report.dirs_removed), (1, 2));
        assert!(report.left_in_place.is_empty());
    }

    /// A sync that only makes `new/deeper` below `base`, as an adapter
    /// creating its target folders with nothing to write into them.
    fn sync_creating_only_directories(base: &Path, root: &Path, now: SystemTime) {
        let session = begin_at(root, now).unwrap();
        crate::adapters::utils::create_dir_all(&base.join("new").join("deeper")).unwrap();
        drop(session);
    }

    #[test]
    fn rollback_removes_the_directories_a_directories_only_sync_created() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        sync_creating_only_directories(tmp.path(), &root, at(SAMPLE));

        let report = rollback(&root, None).unwrap();
        assert!(!tmp.path().join("new").exists());
        assert_eq!(report.dirs_removed, 2);
    }

    #[test]
    fn rerunning_a_directories_only_sync_leaves_no_second_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        sync_creating_only_directories(tmp.path(), &root, at(SAMPLE));
        sync_creating_only_directories(tmp.path(), &root, at(SAMPLE + 60));

        let names: Vec<String> = list(&root).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["20261113T142530Z"]);
    }

    #[test]
    fn rollback_leaves_a_created_directory_the_user_added_files_to() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        sync_creating_directories(tmp.path(), &root);
        let new_dir = tmp.path().join("new");
        std::fs::write(new_dir.join("mine.txt"), b"keep").unwrap();

        let report = rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(new_dir.join("mine.txt")).unwrap(), b"keep");
        assert!(!new_dir.join("deeper").exists());
        assert_eq!(report.dirs_removed, 1);
        assert_eq!(report.left_in_place, [new_dir]);
    }

    #[test]
    fn rollback_skips_a_created_directory_that_is_already_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        sync_creating_directories(tmp.path(), &root);
        std::fs::remove_dir_all(tmp.path().join("new")).unwrap();

        let report = rollback(&root, None).unwrap();
        assert_eq!(report.dirs_removed, 0);
        assert!(report.left_in_place.is_empty());
    }

    #[test]
    fn rollback_leaves_a_file_where_the_sync_had_created_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        sync_creating_directories(tmp.path(), &root);
        let new_dir = tmp.path().join("new");
        std::fs::remove_dir_all(&new_dir).unwrap();
        std::fs::write(&new_dir, b"user file").unwrap();

        let report = rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&new_dir).unwrap(), b"user file");
        assert_eq!(report.left_in_place, [new_dir]);
    }

    #[test]
    fn rollback_keeps_a_directory_that_existed_before_the_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let existing = tmp.path().join("existing");
        std::fs::create_dir(&existing).unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(existing.join("made.md"), b"x").unwrap();
        drop(session);

        let report = rollback(&root, None).unwrap();
        assert!(existing.is_dir());
        assert_eq!(report.dirs_removed, 0);
    }

    #[cfg(unix)]
    #[test]
    fn rollback_recreates_a_symlink_the_sync_removed_with_its_exact_target() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        std::fs::write(tmp.path().join("target"), b"t").unwrap();
        std::os::unix::fs::symlink("target", &link).unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::remove_file(&link).unwrap();
        drop(session);

        let report = rollback(&root, None).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("target"));
        assert_eq!(report.restored, 1);
    }

    #[cfg(unix)]
    #[test]
    fn rollback_replaces_a_file_that_took_a_symlinks_place() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink("elsewhere", &link).unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::remove_file(&link).unwrap();
        write_file(&link, b"plain file").unwrap();
        drop(session);

        rollback(&root, None).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("elsewhere"));
    }

    #[cfg(unix)]
    #[test]
    fn rollback_restores_a_link_a_write_through_a_dangling_chain_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let (first, second) = (tmp.path().join("first"), tmp.path().join("second"));
        std::os::unix::fs::symlink("second", &first).unwrap();
        std::os::unix::fs::symlink("missing", &second).unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&first, b"through").unwrap();
        drop(session);

        rollback(&root, None).unwrap();
        assert_eq!(std::fs::read_link(&first).unwrap(), Path::new("second"));
        assert_eq!(std::fs::read_link(&second).unwrap(), Path::new("missing"));
    }

    #[cfg(unix)]
    #[test]
    fn rollback_undoes_a_copy_through_a_link_to_a_dangling_link() {
        let tmp = tempfile::tempdir().unwrap();
        let (first, second) = (tmp.path().join("first"), tmp.path().join("second"));
        std::os::unix::fs::symlink("second", &first).unwrap();
        std::os::unix::fs::symlink("missing", &second).unwrap();
        let source = tmp.path().join("source.md");
        std::fs::write(&source, b"copied").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::copy(&source, &first).unwrap();
        drop(session);
        assert_eq!(
            std::fs::read(tmp.path().join("missing")).unwrap(),
            b"copied"
        );

        let report = rollback(&root, None).unwrap();
        assert!(report.changed_since_sync.is_empty(), "{report:?}");
        assert_eq!(std::fs::read_link(&first).unwrap(), Path::new("second"));
        assert_eq!(std::fs::read_link(&second).unwrap(), Path::new("missing"));
        assert!(std::fs::symlink_metadata(tmp.path().join("missing")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rollback_refuses_to_replace_a_full_directory_with_a_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink("elsewhere", &link).unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::remove_file(&link).unwrap();
        crate::adapters::utils::create_dir_all(&link).unwrap();
        drop(session);
        std::fs::write(link.join("users-own.txt"), b"keep").unwrap();

        let report = rollback(&root, None).unwrap();
        assert_eq!(report.failed.len(), 1, "{report:?}");
        assert!(report.failed[0].1.contains("not empty"), "{report:?}");
        assert!(link.join("users-own.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rollback_skips_a_special_file_and_reports_it() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("s.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::remove_file(&socket).unwrap();
        drop(session);
        drop(listener);

        let report = rollback(&root, None).unwrap();
        assert_eq!(report.not_restorable, std::slice::from_ref(&socket));
        assert_eq!(report.restored, 0);
        assert!(!socket.exists());
    }

    #[test]
    fn rollback_refuses_a_symlink_entry_without_a_target() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let line = format!(
            "{{\"path\":{},\"prior\":\"file\",\"kind\":\"symlink\"}}\n",
            serde_json::to_string(tmp.path().join("l").to_str().unwrap()).unwrap()
        );
        snapshot_with_manifest(&root, &line);
        let err = rollback(&root, None).unwrap_err();
        assert!(err.to_string().contains("no link target"), "{err}");
    }

    #[test]
    fn rollback_skips_a_file_the_user_edited_after_the_sync_and_reports_it() {
        let tmp = tempfile::tempdir().unwrap();
        let target = drop_a_recorded_change(tmp.path(), at(SAMPLE));
        std::fs::write(&target, b"user edit").unwrap();

        let report = rollback(&tmp.path().join("snaps"), None).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"user edit");
        assert_eq!(report.changed_since_sync, std::slice::from_ref(&target));
        assert_eq!(report.restored, 0);
    }

    #[test]
    fn rollback_keeps_a_directory_the_user_put_where_the_sync_removed_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("asset");
        std::fs::write(&path, b"was a file").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::remove_file(&path).unwrap();
        drop(session);
        std::fs::create_dir(&path).unwrap();

        let report = rollback(&root, None).unwrap();
        assert!(path.is_dir());
        assert_eq!(report.changed_since_sync, [path]);
    }

    #[test]
    fn force_rollback_restores_a_file_the_user_edited_after_the_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let target = drop_a_recorded_change(tmp.path(), at(SAMPLE));
        std::fs::write(&target, b"user edit").unwrap();

        let report = force_rollback(&tmp.path().join("snaps"), None).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"before");
        assert!(report.changed_since_sync.is_empty());
        assert_eq!(report.restored, 1);
    }

    #[test]
    fn rollback_skips_a_file_the_user_deleted_after_the_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let target = drop_a_recorded_change(tmp.path(), at(SAMPLE));
        std::fs::remove_file(&target).unwrap();

        let report = rollback(&tmp.path().join("snaps"), None).unwrap();
        assert!(!target.exists(), "the user's deletion stands");
        assert_eq!(report.changed_since_sync, std::slice::from_ref(&target));
    }

    #[test]
    fn rollback_keeps_a_created_file_the_user_edited_after_the_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let created = tmp.path().join("made.md");
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&created, b"by sync").unwrap();
        drop(session);
        std::fs::write(&created, b"now mine").unwrap();

        let report = rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&created).unwrap(), b"now mine");
        assert_eq!(report.removed, 0);
        assert_eq!(report.changed_since_sync, [created]);
    }

    #[test]
    fn rollback_restores_a_file_the_sync_wrote_then_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("t.txt");
        std::fs::write(&target, b"before").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&target, b"interim").unwrap();
        crate::adapters::utils::remove_file(&target).unwrap();
        drop(session);

        let report = rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"before");
        assert!(report.changed_since_sync.is_empty());
    }

    #[test]
    fn rollback_restores_the_previous_backup_and_the_config() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("settings.json");
        let backup = tmp.path().join("settings.json.skrills-bak");
        std::fs::write(&config, b"v2").unwrap();
        std::fs::write(&backup, b"v1").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::write_config(&config, b"v3", false).unwrap();
        drop(session);
        assert_eq!(std::fs::read(&backup).unwrap(), b"v2");

        rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&config).unwrap(), b"v2");
        assert_eq!(std::fs::read(&backup).unwrap(), b"v1");
    }

    #[test]
    fn rollback_removes_a_backup_the_sync_created() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("settings.json");
        let backup = tmp.path().join("settings.json.skrills-bak");
        std::fs::write(&config, b"v1").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::write_config(&config, b"v2", false).unwrap();
        drop(session);

        rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&config).unwrap(), b"v1");
        assert!(!backup.exists());
    }

    #[test]
    fn manifest_after_line_for_an_unrecorded_path_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = format!(
            "{}{{\"path\":\"/tmp/other\",\"after\":\"absent\"}}\n",
            absent_line("/tmp/a")
        );
        let dir = snapshot_with_manifest(tmp.path(), &manifest);
        let entries = read_manifest(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].after, None);
    }

    #[test]
    fn manifest_line_without_a_kind_reads_as_a_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = snapshot_with_manifest(tmp.path(), &absent_line("/tmp/a"));
        assert_eq!(read_manifest(&dir).unwrap()[0].kind, Kind::File);
    }

    #[test]
    fn rollback_brings_back_every_file_of_a_removed_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let tree = tmp.path().join("skill");
        std::fs::create_dir_all(tree.join("refs")).unwrap();
        std::fs::write(tree.join("SKILL.md"), b"skill").unwrap();
        std::fs::write(tree.join("refs").join("a.md"), b"ref").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        remove_tree(&tree).unwrap();
        drop(session);
        assert!(!tree.exists());

        rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(tree.join("SKILL.md")).unwrap(), b"skill");
        assert_eq!(
            std::fs::read(tree.join("refs").join("a.md")).unwrap(),
            b"ref"
        );
    }

    #[test]
    fn rollback_restores_a_file_a_sync_replaced_with_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("asset");
        std::fs::write(&path, b"was a file").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::remove_file(&path).unwrap();
        write_file(path.join("inner.txt"), b"now a dir").unwrap();
        drop(session);

        rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"was a file");
    }

    #[test]
    fn rollback_refuses_to_replace_a_directory_that_holds_other_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("asset");
        std::fs::write(&path, b"was a file").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::remove_file(&path).unwrap();
        crate::adapters::utils::create_dir_all(&path).unwrap();
        drop(session);
        std::fs::write(path.join("users-own.txt"), b"keep").unwrap();

        let report = rollback(&root, None).unwrap();
        assert_eq!(report.failed.len(), 1, "{report:?}");
        assert_eq!(report.failed[0].0, path);
        assert!(report.failed[0].1.contains("not empty"), "{report:?}");
        assert_eq!(std::fs::read(path.join("users-own.txt")).unwrap(), b"keep");
    }

    #[test]
    fn rollback_leaves_a_directory_that_replaced_a_created_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("made");
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&path, b"x").unwrap();
        drop(session);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir_all(&path).unwrap();

        let report = rollback(&root, None).unwrap();
        assert_eq!(report.removed, 0);
        assert!(path.is_dir());
    }

    #[test]
    fn rollback_skips_a_created_file_that_is_already_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("made");
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&path, b"x").unwrap();
        drop(session);
        std::fs::remove_file(&path).unwrap();

        assert_eq!(rollback(&root, None).unwrap().removed, 0);
    }

    #[test]
    fn rollback_picks_the_newest_snapshot_and_can_pick_an_older_one_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("t.txt");
        let root = tmp.path().join("snaps");
        std::fs::write(&target, b"v1").unwrap();
        let first = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&target, b"v2").unwrap();
        drop(first);
        let second = begin_at(&root, at(SAMPLE + 60)).unwrap();
        write_file(&target, b"v3").unwrap();
        drop(second);

        let newest = rollback(&root, None).unwrap();
        assert_eq!(newest.snapshot, "20261113T142630Z");
        assert_eq!(std::fs::read(&target).unwrap(), b"v2");
        rollback(&root, Some("20261113T142530Z")).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"v1");
    }

    /// Two syncs a minute apart: the first writes `older-only.txt` and
    /// `shared.txt`, the second writes `shared.txt` again. Returns the
    /// snapshot root, the older snapshot's name and both paths.
    fn two_syncs(base: &Path) -> (PathBuf, &'static str, PathBuf, PathBuf) {
        let root = base.join("snaps");
        let (older_only, shared) = (base.join("older-only.txt"), base.join("shared.txt"));
        std::fs::write(&older_only, b"o1").unwrap();
        std::fs::write(&shared, b"v1").unwrap();
        let first = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&older_only, b"o2").unwrap();
        write_file(&shared, b"v2").unwrap();
        drop(first);
        let second = begin_at(&root, at(SAMPLE + 60)).unwrap();
        write_file(&shared, b"v3").unwrap();
        drop(second);
        (root, "20261113T142530Z", older_only, shared)
    }

    #[test]
    fn rollback_of_an_older_snapshot_restores_what_a_newer_sync_rewrote() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, older, older_only, shared) = two_syncs(tmp.path());

        let report = rollback(&root, Some(older)).unwrap();
        assert!(report.changed_since_sync.is_empty(), "{report:?}");
        assert_eq!(std::fs::read(&shared).unwrap(), b"v1");
        assert_eq!(std::fs::read(&older_only).unwrap(), b"o1");
        assert_eq!(report.restored, 2);
    }

    #[test]
    fn rollback_of_an_older_snapshot_keeps_an_edit_made_after_the_newer_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, older, _, shared) = two_syncs(tmp.path());
        std::fs::write(&shared, b"user edit").unwrap();

        let report = rollback(&root, Some(older)).unwrap();
        assert_eq!(std::fs::read(&shared).unwrap(), b"user edit");
        assert_eq!(report.changed_since_sync, std::slice::from_ref(&shared));

        force_rollback(&root, Some(older)).unwrap();
        assert_eq!(std::fs::read(&shared).unwrap(), b"v1");
    }

    #[test]
    fn rollback_of_an_older_snapshot_checks_a_path_no_newer_sync_touched_as_before() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, older, older_only, _) = two_syncs(tmp.path());
        std::fs::write(&older_only, b"user edit").unwrap();

        let report = rollback(&root, Some(older)).unwrap();
        assert_eq!(std::fs::read(&older_only).unwrap(), b"user edit");
        assert_eq!(report.changed_since_sync, std::slice::from_ref(&older_only));
    }

    #[test]
    fn rollback_of_an_older_snapshot_skips_an_unreadable_newer_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let (root, older, _, shared) = two_syncs(tmp.path());
        std::fs::write(
            root.join("20261113T142630Z").join(MANIFEST_FILE),
            b"not json\n",
        )
        .unwrap();

        let report = rollback(&root, Some(older)).unwrap();
        assert_eq!(std::fs::read(&shared).unwrap(), b"v3");
        assert_eq!(report.changed_since_sync, std::slice::from_ref(&shared));
    }

    #[test]
    fn rollback_names_the_snapshot_it_could_not_find() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        drop_a_recorded_change(tmp.path(), at(SAMPLE));
        let err = rollback(&root, Some("20000101T000000Z")).unwrap_err();
        assert!(err
            .to_string()
            .contains("no snapshot named 20000101T000000Z"));
    }

    #[test]
    fn rollback_with_no_snapshots_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let err = rollback(&tmp.path().join("snaps"), None).unwrap_err();
        assert!(err.to_string().contains("no snapshots in"));
    }

    fn snapshot_with_manifest(root: &Path, lines: &str) -> PathBuf {
        let dir = root.join("20261113T142530Z");
        std::fs::create_dir_all(dir.join(BLOB_DIR)).unwrap();
        std::fs::write(dir.join(MANIFEST_FILE), lines).unwrap();
        dir
    }

    fn absent_line(path: &str) -> String {
        format!(
            "{{\"path\":{},\"prior\":\"absent\"}}\n",
            serde_json::to_string(path).unwrap()
        )
    }

    #[test]
    fn rollback_refuses_a_manifest_path_that_escapes_with_parent_components() {
        let tmp = tempfile::tempdir().unwrap();
        let victim = tmp.path().join("victim.txt");
        std::fs::write(&victim, b"not part of any sync").unwrap();
        let escaping = format!("{}/work/../victim.txt", tmp.path().display());
        let root = tmp.path().join("snaps");
        snapshot_with_manifest(&root, &absent_line(&escaping));

        let err = rollback(&root, None).unwrap_err();
        assert!(err.to_string().contains("contains `..`"), "{err}");
        assert!(victim.exists());
    }

    #[test]
    fn rollback_refuses_a_relative_manifest_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        snapshot_with_manifest(&root, &absent_line("relative/file.txt"));
        let err = rollback(&root, None).unwrap_err();
        assert!(err.to_string().contains("not absolute"), "{err}");
    }

    #[test]
    fn rollback_changes_nothing_when_any_entry_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let created = tmp.path().join("created.txt");
        std::fs::write(&created, b"made by sync").unwrap();
        let manifest = format!(
            "{}{}",
            absent_line(created.to_str().unwrap()),
            absent_line("relative")
        );
        let root = tmp.path().join("snaps");
        snapshot_with_manifest(&root, &manifest);

        assert!(rollback(&root, None).is_err());
        assert!(
            created.exists(),
            "the valid entry must not have been applied"
        );
    }

    #[test]
    fn rollback_refuses_a_file_entry_without_a_valid_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let target = tmp.path().join("t");
        for hash in [
            String::new(),
            "../../etc/passwd".to_string(),
            "ABCDEF".repeat(11)[..64].to_string(),
        ] {
            let line = format!(
                "{{\"path\":{},\"prior\":\"file\",\"prior_hash\":{}}}\n",
                serde_json::to_string(target.to_str().unwrap()).unwrap(),
                serde_json::to_string(&hash).unwrap()
            );
            snapshot_with_manifest(&root, &line);
            let err = rollback(&root, None).unwrap_err();
            assert!(
                err.to_string().contains("no valid prior hash"),
                "{hash}: {err}"
            );
        }
    }

    #[test]
    fn rollback_refuses_a_file_entry_with_no_hash_field() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let line = format!(
            "{{\"path\":{},\"prior\":\"file\"}}\n",
            serde_json::to_string(tmp.path().join("t").to_str().unwrap()).unwrap()
        );
        snapshot_with_manifest(&root, &line);
        assert!(rollback(&root, None).is_err());
    }

    #[test]
    fn rollback_refuses_when_the_saved_content_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let line = format!(
            "{{\"path\":{},\"prior\":\"file\",\"prior_hash\":\"{}\"}}\n",
            serde_json::to_string(tmp.path().join("t").to_str().unwrap()).unwrap(),
            hash_content(b"gone")
        );
        snapshot_with_manifest(&root, &line);
        let err = rollback(&root, None).unwrap_err();
        assert!(
            err.to_string().contains("missing the saved content"),
            "{err}"
        );
    }

    #[test]
    fn rollback_refuses_saved_content_that_no_longer_matches_its_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let target = drop_a_recorded_change(tmp.path(), at(SAMPLE));
        let root = tmp.path().join("snaps");
        let blob = root
            .join("20261113T142530Z")
            .join(BLOB_DIR)
            .join(hash_content(b"before"));
        std::fs::write(&blob, b"tampered").unwrap();

        let err = rollback(&root, None).unwrap_err();
        assert!(err.to_string().contains("corrupt"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"after");
    }

    #[test]
    fn manifest_with_a_cut_short_final_line_is_still_usable() {
        let tmp = tempfile::tempdir().unwrap();
        let created = tmp.path().join("created.txt");
        std::fs::write(&created, b"x").unwrap();
        let manifest = format!(
            "{}{{\"path\":\"/tmp/cut",
            absent_line(created.to_str().unwrap())
        );
        let root = tmp.path().join("snaps");
        let dir = snapshot_with_manifest(&root, &manifest);

        assert_eq!(read_manifest(&dir).unwrap().len(), 1);
        // Forced: the entry has no after-line, so a plain rollback would keep
        // the file as changed since the sync.
        assert_eq!(force_rollback(&root, None).unwrap().removed, 1);
    }

    #[test]
    fn manifest_with_a_bad_line_in_the_middle_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = format!(
            "{}not json\n{}",
            absent_line("/tmp/a"),
            absent_line("/tmp/b")
        );
        let dir = snapshot_with_manifest(tmp.path(), &manifest);
        let err = read_manifest(&dir).unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn manifest_with_a_bad_but_terminated_final_line_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = format!("{}not json\n", absent_line("/tmp/a"));
        let dir = snapshot_with_manifest(tmp.path(), &manifest);
        assert!(read_manifest(&dir).is_err());
    }

    #[test]
    fn manifest_blank_lines_are_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = format!("\n{}\n", absent_line("/tmp/a"));
        let dir = snapshot_with_manifest(tmp.path(), &manifest);
        assert_eq!(read_manifest(&dir).unwrap().len(), 1);
    }

    #[test]
    fn manifest_entries_carry_path_prior_state_and_prior_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let target = drop_a_recorded_change(tmp.path(), at(SAMPLE));
        let dir = tmp.path().join("snaps").join("20261113T142530Z");
        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();
        let line: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(line["path"], target.to_str().unwrap());
        assert_eq!(line["prior"], "file");
        assert_eq!(line["prior_hash"], hash_content(b"before"));
    }

    /// Three files rewritten by one sync, in the order `a`, `b`, `c`.
    fn sync_rewriting_three_files(base: &Path, root: &Path) -> [PathBuf; 3] {
        let paths = ["a", "b", "c"].map(|name| base.join(name));
        for (path, name) in paths.iter().zip(["a", "b", "c"]) {
            std::fs::write(path, format!("{name} before")).unwrap();
        }
        let session = begin_at(root, at(SAMPLE)).unwrap();
        for path in &paths {
            write_file(path, b"after").unwrap();
        }
        drop(session);
        paths
    }

    /// Replaces `path` with a directory holding a file of the user's.
    fn replace_with_a_full_directory(path: &Path) {
        std::fs::remove_file(path).unwrap();
        std::fs::create_dir(path).unwrap();
        std::fs::write(path.join("users-own.txt"), b"keep").unwrap();
    }

    #[test]
    fn rollback_that_fails_partway_says_which_paths_it_already_restored() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let [a, b, c] = sync_rewriting_three_files(tmp.path(), &root);
        replace_with_a_full_directory(&b);

        let report = force_rollback(&root, None).unwrap();

        assert_eq!(report.restored, 2, "{report:?}");
        assert_eq!(std::fs::read(&a).unwrap(), b"a before");
        assert_eq!(std::fs::read(&c).unwrap(), b"c before");
        assert_eq!(report.failed.len(), 1, "{report:?}");
        assert_eq!(report.failed[0].0, b);
        assert!(report.failed[0].1.contains("not empty"), "{report:?}");
        assert_eq!(std::fs::read(b.join("users-own.txt")).unwrap(), b"keep");
    }

    #[test]
    fn rollback_with_a_corrupt_saved_file_changes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let paths = sync_rewriting_three_files(tmp.path(), &root);
        let blob = root
            .join("20261113T142530Z")
            .join(BLOB_DIR)
            .join(hash_content(b"b before"));
        std::fs::write(&blob, b"tampered").unwrap();

        let err = rollback(&root, None).unwrap_err();

        assert!(err.to_string().contains("corrupt"), "{err}");
        assert!(
            err.to_string().contains(paths[1].to_str().unwrap()),
            "{err}"
        );
        for path in &paths {
            assert_eq!(std::fs::read(path).unwrap(), b"after", "{}", path.display());
        }
    }

    #[test]
    fn rollback_rerun_after_a_partial_failure_does_not_call_restored_paths_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let [a, b, c] = sync_rewriting_three_files(tmp.path(), &root);
        replace_with_a_full_directory(&b);
        force_rollback(&root, None).unwrap();

        let report = rollback(&root, None).unwrap();

        assert!(!report.changed_since_sync.contains(&a), "{report:?}");
        assert!(!report.changed_since_sync.contains(&c), "{report:?}");
        assert_eq!(report.changed_since_sync, std::slice::from_ref(&b));
    }

    #[test]
    fn rollback_refuses_a_link_or_special_entry_recorded_as_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        for kind in ["symlink", "special"] {
            let line = format!(
                "{{\"path\":{},\"prior\":\"absent\",\"kind\":\"{kind}\",\"link_target\":\"x\"}}\n",
                serde_json::to_string(tmp.path().join("l").to_str().unwrap()).unwrap()
            );
            snapshot_with_manifest(&root, &line);
            let err = rollback(&root, None).unwrap_err();
            assert!(
                err.to_string().contains("recorded as absent"),
                "{kind}: {err}"
            );
        }
    }

    /// A manifest entry for `target` with prior bytes `old` and no
    /// after-line, as a sync leaves it when recording the after-state failed.
    fn entry_without_an_after_line(root: &Path, target: &Path) {
        let dir = snapshot_with_manifest(
            root,
            &format!(
                "{{\"path\":{},\"prior\":\"file\",\"prior_hash\":\"{}\"}}\n",
                serde_json::to_string(target.to_str().unwrap()).unwrap(),
                hash_content(b"old")
            ),
        );
        std::fs::write(dir.join(BLOB_DIR).join(hash_content(b"old")), b"old").unwrap();
    }

    #[test]
    fn rollback_keeps_an_edit_to_a_path_whose_after_line_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("t");
        std::fs::write(&target, b"edited by anyone").unwrap();
        let root = tmp.path().join("snaps");
        entry_without_an_after_line(&root, &target);

        let report = rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"edited by anyone");
        assert_eq!(report.changed_since_sync, std::slice::from_ref(&target));

        force_rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
    }

    #[test]
    fn rollback_does_not_report_a_path_without_an_after_line_that_holds_its_prior_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("t");
        std::fs::write(&target, b"old").unwrap();
        let root = tmp.path().join("snaps");
        entry_without_an_after_line(&root, &target);

        let report = rollback(&root, None).unwrap();
        assert!(report.changed_since_sync.is_empty(), "{report:?}");
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
    }

    #[cfg(unix)]
    #[test]
    fn force_rollback_replaces_a_link_at_the_path_instead_of_writing_through_it() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("settings.json");
        std::fs::write(&config, b"{\"token\":1}").unwrap();
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&config, b"{}").unwrap();
        drop(session);
        let dotfiles = tmp.path().join("dotfiles-settings.json");
        std::fs::write(&dotfiles, b"dotfiles").unwrap();
        std::fs::set_permissions(&dotfiles, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::remove_file(&config).unwrap();
        std::os::unix::fs::symlink(&dotfiles, &config).unwrap();

        force_rollback(&root, None).unwrap();

        let mode = |p: &Path| std::fs::symlink_metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(std::fs::read(&dotfiles).unwrap(), b"dotfiles");
        assert_eq!(mode(&dotfiles), 0o644);
        assert!(std::fs::symlink_metadata(&config).unwrap().is_file());
        assert_eq!(std::fs::read(&config).unwrap(), b"{\"token\":1}");
        assert_eq!(mode(&config), 0o600);
    }

    #[test]
    fn rollback_restores_a_file_the_sync_replaced_with_a_nested_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("asset");
        std::fs::write(&path, b"was a file").unwrap();
        let root = tmp.path().join("snaps");
        let session = begin_at(&root, at(SAMPLE)).unwrap();
        crate::adapters::utils::remove_file(&path).unwrap();
        write_file(path.join("sub").join("deeper").join("inner.txt"), b"x").unwrap();
        drop(session);

        let report = rollback(&root, None).unwrap();
        assert!(report.failed.is_empty(), "{report:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"was a file");
        assert_eq!(report.dirs_removed, 2, "{report:?}");
    }

    /// A finished snapshot at [`SAMPLE`], and a session a minute later that
    /// has recorded a change and is still open.
    fn finished_and_open_snapshot(base: &Path, root: &Path) -> (PathBuf, WriteSession) {
        let target = base.join("t.txt");
        std::fs::write(&target, b"v1").unwrap();
        let first = begin_at(root, at(SAMPLE)).unwrap();
        write_file(&target, b"v2").unwrap();
        drop(first);
        let open = begin_at(root, at(SAMPLE + 60)).unwrap();
        write_file(&target, b"v3").unwrap();
        (target, open)
    }

    #[test]
    fn rollback_skips_a_snapshot_whose_session_is_still_open() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let (_target, open) = finished_and_open_snapshot(tmp.path(), &root);

        let report = rollback(&root, None);
        drop(open);

        assert_eq!(report.unwrap().snapshot, "20261113T142530Z");
    }

    #[test]
    fn rollback_refuses_a_named_snapshot_whose_session_is_still_open() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let (target, open) = finished_and_open_snapshot(tmp.path(), &root);

        let refused = rollback(&root, Some("20261113T142630Z"));
        drop(open);

        let err = refused.unwrap_err();
        assert!(err.to_string().contains("still running"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"v3");
    }

    #[test]
    fn retention_zero_does_not_prune_an_open_session() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let (_target, open) = finished_and_open_snapshot(tmp.path(), &root);

        let pruned = prune(&root, at(SAMPLE + 86_400), Duration::ZERO);
        let open_survived = root.join("20261113T142630Z").is_dir();
        drop(open);

        assert_eq!(pruned.unwrap(), 1);
        assert!(open_survived);
        assert!(!root.join("20261113T142530Z").exists());
    }

    #[test]
    fn rollback_with_only_an_open_session_says_its_sync_is_still_running() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let target = tmp.path().join("t.txt");
        std::fs::write(&target, b"v1").unwrap();
        let open = begin_at(&root, at(SAMPLE)).unwrap();
        write_file(&target, b"v2").unwrap();

        let refused = rollback(&root, None);
        drop(open);

        let err = refused.unwrap_err();
        assert!(err.to_string().contains("still running"), "{err}");
        assert_eq!(std::fs::read(&target).unwrap(), b"v2");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_entry_without_an_after_line_is_checked_against_its_prior_target() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("l");
        let root = tmp.path().join("snaps");
        snapshot_with_manifest(
            &root,
            &format!(
                "{{\"path\":{},\"prior\":\"file\",\"kind\":\"symlink\",\"link_target\":\"there\"}}\n",
                serde_json::to_string(link.to_str().unwrap()).unwrap()
            ),
        );

        let report = rollback(&root, None).unwrap();
        assert_eq!(report.changed_since_sync, std::slice::from_ref(&link));
        assert!(std::fs::symlink_metadata(&link).is_err());

        force_rollback(&root, None).unwrap();
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("there"));

        let report = rollback(&root, None).unwrap();
        assert!(report.changed_since_sync.is_empty(), "{report:?}");
        assert_eq!(report.restored, 1);
    }

    #[test]
    fn pruning_leaves_a_snapshot_that_a_rollback_holds() {
        let tmp = tempfile::tempdir().unwrap();
        drop_a_recorded_change(tmp.path(), at(SAMPLE));
        let root = tmp.path().join("snaps");
        let Claim::Free(held) = try_claim(&root.join("20261113T142530Z"), true).unwrap() else {
            panic!("a finished snapshot is free");
        };

        let pruned = prune(&root, at(SAMPLE + 86_400), Duration::ZERO);
        drop(held);

        assert_eq!(pruned.unwrap(), 0);
        assert!(root.join("20261113T142530Z").is_dir());
    }

    #[test]
    fn a_closed_session_is_rolled_back_and_pruned_as_usual() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        let (_target, open) = finished_and_open_snapshot(tmp.path(), &root);
        drop(open);

        assert_eq!(rollback(&root, None).unwrap().snapshot, "20261113T142630Z");
        assert_eq!(
            prune(&root, at(SAMPLE + 86_400), Duration::ZERO).unwrap(),
            2
        );
    }

    #[test]
    fn snapshots_from_the_same_second_list_in_numeric_suffix_order() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        for name in [
            "20261113T142530Z-10",
            "20261113T142530Z",
            "20261113T142530Z-2",
        ] {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
        let names: Vec<_> = list(&root).unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            [
                "20261113T142530Z-10",
                "20261113T142530Z-2",
                "20261113T142530Z"
            ]
        );
    }

    #[test]
    fn prune_keeps_a_snapshot_exactly_at_the_retention_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("snaps");
        std::fs::create_dir_all(root.join("20261113T142530Z")).unwrap();
        let keep = Duration::from_secs(30 * 86_400);
        assert_eq!(prune(&root, at(SAMPLE) + keep, keep).unwrap(), 0);
        assert_eq!(
            prune(&root, at(SAMPLE) + keep + Duration::from_secs(1), keep).unwrap(),
            1
        );
    }
}
