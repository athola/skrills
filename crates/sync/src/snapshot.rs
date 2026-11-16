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
//! appended when two syncs start in the same second. Names sort in time
//! order, and retention reads the age from the name rather than the
//! filesystem, so copying or restoring a snapshot directory cannot make it
//! look new. A directory whose name does not parse is not ours and is never
//! listed, rolled back or pruned.

use crate::adapters::utils::{atomic_write, hash_content, remove_dir, remove_file};
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
    /// A regular file; its bytes are in `blobs/<prior_hash>`.
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
    /// `None` when the manifest has none (an older manifest, or a write that
    /// failed); such a path is restored without checking what is there now.
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
    /// Paths that no longer hold what the sync left, so someone changed
    /// them since; they are kept as they are. [`force_rollback`] restores
    /// them too.
    pub changed_since_sync: Vec<PathBuf>,
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
/// session it only removes the tree.
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
    snapshots.sort_by(|a, b| (b.taken_at, &b.name).cmp(&(a.taken_at, &a.name)));
    Ok(snapshots)
}

/// Deletes snapshots under `root` taken more than `keep` before `now`, and
/// returns how many went. One that cannot be deleted is logged and left.
pub fn prune(root: &Path, now: SystemTime, keep: Duration) -> Result<usize> {
    let mut pruned = 0;
    for snapshot in list(root)? {
        let age = now.duration_since(snapshot.taken_at).unwrap_or_default();
        if age <= keep {
            continue;
        }
        match crate::adapters::utils::remove_tree_unrecorded(&root.join(&snapshot.name)) {
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

/// Restores `name`, or the newest snapshot when `name` is `None`.
///
/// Every manifest entry is checked before the first file changes, so a
/// manifest naming a path outside what a sync could have written (relative,
/// or containing `..`) refuses the whole rollback instead of half of it.
/// Entries are undone newest first, so a path a sync replaced with a
/// directory, or the reverse, comes back in the order it was lost.
/// Directories the sync created go last, deepest first, once the files in
/// them are gone; one that still holds anything is left and reported.
///
/// A path that no longer holds what the sync left there was changed since,
/// and is kept and listed in [`RollbackReport::changed_since_sync`].
pub fn rollback(root: &Path, name: Option<&str>) -> Result<RollbackReport> {
    rollback_checked(root, name, false)
}

/// [`rollback`] that also restores paths changed since the sync, discarding
/// those changes.
pub fn force_rollback(root: &Path, name: Option<&str>) -> Result<RollbackReport> {
    rollback_checked(root, name, true)
}

fn rollback_checked(root: &Path, name: Option<&str>, force: bool) -> Result<RollbackReport> {
    let mut snapshots = list(root)?.into_iter();
    let chosen = match name {
        Some(wanted) => snapshots
            .find(|snapshot| snapshot.name == wanted)
            .ok_or_else(|| anyhow!("no snapshot named {wanted} in {}", root.display()))?,
        None => snapshots
            .next()
            .ok_or_else(|| anyhow!("no snapshots in {}", root.display()))?,
    };
    let dir = root.join(&chosen.name);
    let entries = read_manifest(&dir)?;
    let blobs = entries
        .iter()
        .map(|entry| checked_blob(&dir, entry))
        .collect::<Result<Vec<_>>>()?;

    let (mut restored, mut removed) = (0, 0);
    let mut created_dirs = Vec::new();
    let mut not_restorable = Vec::new();
    let mut changed_since_sync = Vec::new();
    for (entry, blob) in entries.iter().zip(&blobs).rev() {
        if entry.kind == Kind::Dir {
            created_dirs.push(entry.path.as_path());
            continue;
        }
        if let Some(left_by_sync) = &entry.after {
            if !force && current_state(&entry.path)?.as_ref() != Some(left_by_sync) {
                changed_since_sync.push(entry.path.clone());
                continue;
            }
        }
        match entry.kind {
            Kind::Dir => continue,
            Kind::Special => {
                tracing::warn!(path = %entry.path.display(), "Cannot recreate a special file");
                not_restorable.push(entry.path.clone());
                continue;
            }
            Kind::Symlink => {
                #[cfg(unix)]
                {
                    restore_link(entry)?;
                    restored += 1;
                }
                #[cfg(not(unix))]
                not_restorable.push(entry.path.clone());
                continue;
            }
            Kind::File => {}
        }
        match blob {
            Some(blob) => {
                restore_file(entry, blob)?;
                restored += 1;
            }
            None => {
                if delete_created(&entry.path)? {
                    removed += 1;
                }
            }
        }
    }
    created_dirs.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    let mut dirs_removed = 0;
    let mut left_in_place = Vec::new();
    for dir in created_dirs {
        if remove_created_dir(dir)? {
            dirs_removed += 1;
        } else if dir.symlink_metadata().is_ok() {
            left_in_place.push(dir.to_path_buf());
        }
    }
    Ok(RollbackReport {
        snapshot: chosen.name,
        restored,
        removed,
        dirs_removed,
        left_in_place,
        not_restorable,
        changed_since_sync,
    })
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
    Ok(Some(blob))
}

fn restore_file(entry: &Entry, blob: &Path) -> Result<()> {
    let bytes =
        std::fs::read(blob).with_context(|| format!("Failed to read {}", blob.display()))?;
    if Some(hash_content(&bytes).as_str()) != entry.prior_hash.as_deref() {
        bail!("saved content of {} is corrupt", entry.path.display());
    }
    // A sync that replaced this file with a directory has had its own files
    // removed already; what is left must be empty, or it is the user's.
    if std::fs::symlink_metadata(&entry.path).is_ok_and(|meta| meta.is_dir()) {
        remove_dir(&entry.path).with_context(|| {
            format!(
                "{} is now a directory that is not empty",
                entry.path.display()
            )
        })?;
    }
    atomic_write(&entry.path, &bytes, false)
        .with_context(|| format!("Failed to restore {}", entry.path.display()))?;
    #[cfg(unix)]
    if let Some(mode) = entry.mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&entry.path, std::fs::Permissions::from_mode(mode))
            .with_context(|| format!("Failed to restore the mode of {}", entry.path.display()))?;
    }
    Ok(())
}

/// Puts back a symlink the sync removed or replaced, pointing exactly where it
/// pointed before.
#[cfg(unix)]
fn restore_link(entry: &Entry) -> Result<()> {
    let target = entry.link_target.as_deref().ok_or_else(|| {
        anyhow!(
            "manifest entry for {} is a symlink with no link target",
            entry.path.display()
        )
    })?;
    // As for a file: an empty directory in its place is the sync's leftover,
    // a full one is the user's.
    if std::fs::symlink_metadata(&entry.path).is_ok_and(|meta| meta.is_dir()) {
        remove_dir(&entry.path).with_context(|| {
            format!(
                "{} is now a directory that is not empty",
                entry.path.display()
            )
        })?;
    }
    crate::adapters::utils::replace_with_symlink(&entry.path, target)
        .with_context(|| format!("Failed to restore the link {}", entry.path.display()))
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

        let err = rollback(&root, None).unwrap_err();
        assert!(format!("{err:#}").contains("not empty"), "{err:#}");
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
    fn rollback_of_a_manifest_without_after_hashes_restores_whatever_is_there() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("t");
        std::fs::write(&target, b"edited by anyone").unwrap();
        let root = tmp.path().join("snaps");
        let dir = snapshot_with_manifest(
            &root,
            &format!(
                "{{\"path\":{},\"prior\":\"file\",\"prior_hash\":\"{}\"}}\n",
                serde_json::to_string(target.to_str().unwrap()).unwrap(),
                hash_content(b"old")
            ),
        );
        std::fs::write(dir.join(BLOB_DIR).join(hash_content(b"old")), b"old").unwrap();

        let report = rollback(&root, None).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
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

        let err = rollback(&root, None).unwrap_err();
        assert!(format!("{err:#}").contains("not empty"), "{err:#}");
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
        assert_eq!(rollback(&root, None).unwrap().removed, 1);
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
}
