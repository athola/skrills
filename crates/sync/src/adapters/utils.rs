//! Shared utility functions for agent adapters.

use crate::common::ModuleFile;
use crate::error::SyncError;
use crate::Result;
use sha2::{Digest, Sha256};
use skrills_snapshot::KillSwitch;
use std::path::Path;
use tracing::{debug, warn};
use walkdir::WalkDir;

/// Refuse a mutating sync operation when the cold-window kill-switch is engaged.
///
/// Adapters call this at the top of every `write_*` method. When the switch
/// is `None` (no engine present, e.g. unit tests of the adapter in isolation)
/// or disengaged, the call is a near-zero-cost atomic load.
///
/// On engagement, returns [`SyncError::KillSwitchEngaged`] wrapped in
/// `anyhow::Error`. The token numbers live in the engine that engaged the
/// switch, which reports them in its own WARNING-tier alert.
pub(crate) fn ensure_not_engaged(switch: Option<&KillSwitch>) -> Result<()> {
    if let Some(s) = switch {
        if s.is_engaged() {
            return Err(SyncError::KillSwitchEngaged.into());
        }
    }
    Ok(())
}

/// Reads one item file a reader found, with its modification time.
///
/// An unreadable file is skipped with a warning: failing the read would fail
/// the whole phase on every run, and the Claude readers already skip.
pub(crate) fn read_item_file(path: &Path) -> Option<(Vec<u8>, std::time::SystemTime)> {
    let read = std::fs::read(path).and_then(|content| Ok((content, std::fs::metadata(path)?)));
    match read {
        Ok((content, meta)) => Some((
            content,
            meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        )),
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Skipping unreadable file");
            None
        }
    }
}

/// Returns true if the name starts with a dot (hidden file/directory).
pub fn is_hidden_component(name: &str) -> bool {
    name.starts_with('.')
}

/// Returns true if any path component is hidden (starts with a dot).
pub fn is_hidden_path(path: &Path) -> bool {
    path.components().any(|c| match c {
        std::path::Component::Normal(s) => is_hidden_component(&s.to_string_lossy()),
        _ => false,
    })
}

/// Checks whether `target_path` stays within `base_dir` after resolution.
///
/// Returns `true` if the path is contained, `false` if it escapes (path traversal).
/// For files that don't exist yet, canonicalizes the base directory and checks
/// that the relative path has no `..` components that escape the base.
pub fn is_path_contained(target_path: &Path, base_dir: &Path) -> bool {
    // Fast path: if both exist on disk, use canonical resolution
    if let (Ok(resolved), Ok(canonical_base)) =
        (target_path.canonicalize(), base_dir.canonicalize())
    {
        return resolved.starts_with(&canonical_base);
    }

    // Target doesn't exist yet, try canonicalizing the parent chain
    let resolved = target_path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.join(target_path.file_name().unwrap_or_default()));

    if let Some(resolved) = resolved {
        let canonical_base = base_dir
            .canonicalize()
            .unwrap_or_else(|_| base_dir.to_path_buf());
        return resolved.starts_with(&canonical_base);
    }

    // Neither target nor its parent exist, check if base_dir exists and
    // verify that the relative path between them has no traversal
    let canonical_base = match base_dir.canonicalize() {
        Ok(b) => b,
        Err(_) => return false, // Base doesn't exist, deny
    };

    // If target_path starts with base_dir, strip the prefix and check for ..
    if let Ok(relative) = target_path.strip_prefix(base_dir) {
        // No component should be ".."
        return !relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    }

    // If target_path is absolute but doesn't start with base_dir, try the
    // canonical base
    if let Ok(relative) = target_path.strip_prefix(&canonical_base) {
        return !relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    }

    false // Cannot determine containment, deny (fail-closed)
}

/// Computes a SHA-256 hash of the given content, returning a lowercase hex string.
pub fn hash_content(content: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hex::encode(hasher.finalize())
}

/// Sanitizes a name by filtering to safe characters: alphanumeric, hyphens, and underscores.
///
/// Prevents path traversal by stripping dots, slashes, and other special characters.
/// Use `sanitize_name_segments` for names that may contain legitimate path separators.
pub fn sanitize_name(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

/// Sanitizes a name while preserving forward-slash path segments.
///
/// Each segment is individually filtered to `[a-zA-Z0-9_-]`.
/// Empty segments and traversal segments (`.` and `..`) are removed.
pub fn sanitize_name_segments(name: &str) -> String {
    name.split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .map(|segment| {
            segment
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Largest companion or plugin asset file sync will read. Every file is held
/// in memory for the whole sync, so an unbounded read of a stray model
/// checkpoint or log inside a skill directory could exhaust memory.
pub(crate) const MAX_MODULE_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Collects companion files from a skill directory (files other than SKILL.md).
///
/// Symlinks are not followed: a link inside a skill can point anywhere, and
/// following it copied its target into every other tool's directory. Files
/// over [`MAX_MODULE_FILE_BYTES`] are skipped with a warning.
pub fn collect_module_files(skill_dir: &Path) -> Vec<ModuleFile> {
    let mut modules = Vec::new();

    // A subdirectory holding its own SKILL.md is a separate skill, not a
    // module of this one.
    for entry in WalkDir::new(skill_dir)
        .min_depth(1)
        .max_depth(10)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && e.path().join("SKILL.md").is_file()))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                debug!(
                    error = %e,
                    path = ?e.path(),
                    "Skipping directory entry due to traversal error"
                );
                continue;
            }
        };

        let path = entry.path();

        // `entry.file_type()` does not follow links, unlike `path.is_file()`.
        if !entry.file_type().is_file() {
            continue;
        }

        if path == skill_dir.join("SKILL.md") {
            continue;
        }

        if let Ok(rel_path) = path.strip_prefix(skill_dir) {
            if is_hidden_path(rel_path) {
                continue;
            }

            if let Ok(meta) = entry.metadata() {
                if meta.len() > MAX_MODULE_FILE_BYTES {
                    warn!(
                        path = %path.display(),
                        bytes = meta.len(),
                        limit = MAX_MODULE_FILE_BYTES,
                        "Skipping module file over the size limit"
                    );
                    continue;
                }
            }
            let content = match std::fs::read(path) {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        path = %path.display(),
                        error = %e,
                        "Skipping unreadable module file"
                    );
                    continue;
                }
            };
            let hash = hash_content(&content);
            modules.push(ModuleFile {
                relative_path: rel_path.to_path_buf(),
                content,
                hash,
            });
        }
    }

    modules
}

/// Sanitizes a name to kebab-case suitable for file/directory names.
///
/// Lowercases, maps `_`/` `/`.` to hyphens, strips other non-alphanumeric characters,
/// and trims leading/trailing hyphens. Used by the Cursor adapter since Cursor
/// conventions use kebab-case file names.
///
/// Note: a round-trip through a kebab-case adapter will normalize names
/// (e.g., `My_Skill` → `my-skill`), so the returned name may differ from the input.
pub fn sanitize_name_kebab(name: &str) -> String {
    let raw: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else if c == '_' || c == ' ' || c == '.' {
                '-'
            } else {
                '\0'
            }
        })
        .filter(|&c| c != '\0')
        .collect();
    // Collapse consecutive hyphens and trim
    let mut result = String::with_capacity(raw.len());
    for c in raw.chars() {
        if c == '-' && result.ends_with('-') {
            continue;
        }
        result.push(c);
    }
    result.trim_matches('-').to_string()
}

/// Suffix of the one-generation backup [`write_config`] keeps beside a user
/// config file it is about to change.
pub(crate) const BACKUP_SUFFIX: &str = ".skrills-bak";

/// Distinguishes temp files made by concurrent writes in one process.
static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Replaces `path` with `bytes` so a reader sees either the old file or the
/// new one, never a truncated or interleaved mix.
///
/// The bytes go to a hidden sibling temp file that is synced and then renamed
/// over the destination. A destination that is a symlink (a dotfiles checkout,
/// say) is resolved first so the link survives and its target is replaced. An
/// existing file keeps its permission bits; a new one gets `0o600` when
/// `private` is set and the umask default otherwise.
///
/// The destination's prior state goes to the active
/// [`write_session`](crate::write_session) first, so a snapshot can restore it.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    let dest = resolve_destination(path)?;
    if let Some(parent) = dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        crate::write_session::record_dirs_before_create(parent)?;
    }
    crate::write_session::record_before_write(&dest)?;
    atomic_write_unrecorded(path, bytes, private)?;
    crate::write_session::record_after_write(&dest, bytes)
}

/// [`std::fs::create_dir_all`] that first records each directory it is about
/// to create, so a rollback can remove the ones the sync added.
pub(crate) fn create_dir_all(path: &Path) -> std::io::Result<()> {
    crate::write_session::record_dirs_before_create(path)?;
    std::fs::create_dir_all(path)
}

/// Where a write to `path` lands: the path itself, or the end of the symlink
/// chain at `path`.
///
/// A chain ending in a missing file resolves to that missing file, as opening
/// it would, so every link in the chain survives the rename. A loop, or a
/// chain longer than [`MAX_LINK_HOPS`], is an error.
fn resolve_destination(path: &Path) -> std::io::Result<std::path::PathBuf> {
    let mut resolved = path.to_path_buf();
    for _ in 0..MAX_LINK_HOPS {
        match std::fs::symlink_metadata(&resolved) {
            Ok(meta) if meta.file_type().is_symlink() => {}
            _ => return Ok(resolved),
        }
        if let Ok(real) = std::fs::canonicalize(&resolved) {
            return Ok(real);
        }
        // A dangling hop: follow it by hand, so the link survives.
        let target = std::fs::read_link(&resolved)?;
        resolved = match resolved.parent() {
            Some(parent) if target.is_relative() => parent.join(target),
            _ => target,
        };
    }
    Err(std::io::Error::other(format!(
        "{} is a symlink loop or a chain of more than {MAX_LINK_HOPS} links",
        path.display()
    )))
}

/// [`atomic_write`] without telling the snapshot recorder. For the files the
/// snapshot itself keeps and for skrills' own sync state, which must not be
/// recorded as something a sync changed.
pub(crate) fn atomic_write_unrecorded(
    path: &Path,
    bytes: &[u8],
    private: bool,
) -> std::io::Result<()> {
    use std::io::Write;

    let dest = resolve_destination(path)?;
    let parent = match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    std::fs::create_dir_all(&parent)?;

    let file_name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(
        ".{file_name}.skrills-tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));

    let existing_perms = std::fs::metadata(&dest).ok().map(|m| m.permissions());

    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(if private { 0o600 } else { 0o666 });
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        if let Some(perms) = existing_perms {
            file.set_permissions(perms)?;
        }
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &dest)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }

    // Persist the rename itself; failure here does not undo the write.
    #[cfg(unix)]
    if let Ok(dir) = std::fs::File::open(&parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// [`atomic_write`] with the call shape of `std::fs::write`, for generated
/// files (skills, commands, agents, rules) that need no backup.
///
/// A file that already holds `bytes` is left alone, so a sync that changes
/// nothing records nothing and leaves no snapshot for a rollback to pick.
pub(crate) fn write_file(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    let (path, bytes) = (path.as_ref(), bytes.as_ref());
    if std::fs::read(path).is_ok_and(|existing| existing == bytes) {
        return Ok(());
    }
    atomic_write(path, bytes, false)
}

/// Removes one file, recording its content first so a snapshot can restore it.
pub(crate) fn remove_file(path: &Path) -> std::io::Result<()> {
    crate::write_session::record_before_write(path)?;
    std::fs::remove_file(path)?;
    crate::write_session::record_after_remove(path)
}

/// Removes a directory tree, recording every file in it first so a snapshot
/// can restore them.
pub(crate) fn remove_tree(path: &Path) -> std::io::Result<()> {
    let removed = crate::write_session::record_tree_before_remove(path)?;
    std::fs::remove_dir_all(path)?;
    for file in removed {
        crate::write_session::record_after_remove(&file)?;
    }
    Ok(())
}

/// [`remove_tree`] for the snapshot's own storage, which is never recorded.
pub(crate) fn remove_tree_unrecorded(path: &Path) -> std::io::Result<()> {
    std::fs::remove_dir_all(path)
}

/// Puts a symlink to `target` at `path`, replacing a file or link there with
/// one rename so the path is never missing in between. Rollback uses it to
/// bring back a link exactly as it was, relative targets included.
#[cfg(unix)]
pub(crate) fn replace_with_symlink(path: &Path, target: &Path) -> std::io::Result<()> {
    crate::write_session::record_before_write(path)?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    create_dir_all(&parent)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(
        ".{file_name}.skrills-tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::os::unix::fs::symlink(target, &tmp)?;
    let renamed = std::fs::rename(&tmp, path);
    if renamed.is_err() {
        // The rename error is the one worth reporting; a leftover temp link
        // is hidden and harmless.
        let _ = std::fs::remove_file(&tmp);
    }
    renamed
}

/// Puts `bytes` at `path` itself with permission bits `mode`, for rollback.
///
/// Unlike [`atomic_write`], a symlink at `path` is replaced, not written
/// through, and the temp file is created `0o600` and given `mode` before the
/// rename, so the content is never readable under looser bits. `mode` is
/// ignored off Unix; `None` keeps the umask default. Nothing is recorded:
/// rollback runs outside a session.
pub(crate) fn replace_file_unrecorded(
    path: &Path,
    bytes: &[u8],
    mode: Option<u32>,
) -> std::io::Result<()> {
    use std::io::Write;

    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => std::path::PathBuf::from("."),
    };
    std::fs::create_dir_all(&parent)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = parent.join(format!(
        ".{file_name}.skrills-tmp-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(if mode.is_some() { 0o600 } else { 0o666 });
        }
        let mut file = options.open(&tmp)?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        // The write error is the one worth reporting; a leftover temp file
        // is hidden and is removed here when it can be.
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Removes an empty directory. Nothing is recorded: a directory that holds
/// no file has nothing a rollback needs to bring back, and the OS refuses a
/// non-empty one.
pub(crate) fn remove_dir(path: &Path) -> std::io::Result<()> {
    std::fs::remove_dir(path)
}

/// [`std::fs::copy`] that records the destination's prior state first.
///
/// `std::fs::copy` opens a destination symlink and writes into what it points
/// to, so the resolved target is what gets recorded, as [`atomic_write`] does.
pub(crate) fn copy(src: &Path, dest: &Path) -> std::io::Result<u64> {
    let resolved = resolve_destination(dest)?;
    crate::write_session::record_before_write(&resolved)?;
    let copied = std::fs::copy(src, dest)?;
    crate::write_session::record_after_copy(&resolved)?;
    Ok(copied)
}

/// Sets `path`'s permission bits to `mode`, recording its bytes and old mode
/// first so a rollback can put the mode back. A mode-only change has no
/// other trace in the snapshot.
#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // `set_permissions` follows links, so the file it changes is the one
    // recorded.
    let resolved = resolve_destination(path)?;
    crate::write_session::record_before_write(&resolved)?;
    std::fs::set_permissions(&resolved, std::fs::Permissions::from_mode(mode))?;
    crate::write_session::record_after_copy(&resolved)
}

/// Most links the kernel follows before giving up with `ELOOP` on Linux.
const MAX_LINK_HOPS: usize = 40;

/// Writes a user-owned config file (`settings.json`, `CLAUDE.md`, `mcp.json`,
/// `config.toml`, ...): no-op when the content is unchanged, otherwise keeps a
/// one-generation copy of the previous content at `<path>.skrills-bak` and
/// replaces the file atomically.
///
/// Returns whether the file was written. Backups are only kept for config
/// files: a `.skrills-bak` beside a skill would be picked up as one of its
/// module files on the next sync.
///
/// The config is recorded before its backup is written, so a sync cut short
/// between the two still has the config in its snapshot. The backup write is
/// recorded too: rollback puts back the previous backup, or removes one this
/// sync created.
pub(crate) fn write_config(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<bool> {
    match std::fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(false),
        Ok(existing) => {
            crate::write_session::record_before_write(&resolve_destination(path)?)?;
            let mut backup = path.as_os_str().to_owned();
            backup.push(BACKUP_SUFFIX);
            // The backup holds the same secrets as the original, so it is
            // always private.
            atomic_write(Path::new(&backup), &existing, true)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    atomic_write(path, bytes, private)?;
    Ok(true)
}

/// Returns the first symlink on the way from `root` (exclusive) down to
/// `path` (inclusive), if any.
///
/// `~/.claude/skills/foo` is often a symlink into a git checkout. Writing
/// through it overwrote the user's working tree with another tool's copy.
pub(crate) fn symlink_below(root: &Path, path: &Path) -> Option<std::path::PathBuf> {
    let relative = path.strip_prefix(root).ok()?;
    let mut walked = root.to_path_buf();
    for component in relative.components() {
        walked.push(component);
        match std::fs::symlink_metadata(&walked) {
            Ok(meta) if meta.file_type().is_symlink() => return Some(walked),
            Ok(_) => continue,
            Err(_) => return None,
        }
    }
    None
}

/// The directory name a skill is written under: its name, or for a skill whose
/// name is just `SKILL`/`skill.md`, the name of the directory it came from.
pub(crate) fn skill_dir_name(skill: &crate::common::Command) -> String {
    let generic = ["skill", "skill.md"]
        .iter()
        .any(|g| skill.name.eq_ignore_ascii_case(g));
    if generic {
        skill
            .source_path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or(&skill.name)
            .to_string()
    } else {
        skill.name.clone()
    }
}

/// The folder a skill is written to in every target: the last segment of its
/// name, kebab-cased, so `synced/<id>/morning` lands in `morning/` whichever
/// tool receives it, and `a/b` and `ab` stay apart.
///
/// One segment because Cursor and OpenCode load a skill only when its
/// frontmatter `name` matches its folder, and the shared `~/.agents/skills`
/// is read one level deep.
pub(crate) fn skill_folder_name(skill: &crate::common::Command) -> String {
    let dir = skill_dir_name(skill);
    let leaf = dir.rsplit(['/', '\\']).next().unwrap_or(&dir);
    sanitize_name_kebab(leaf)
}

/// File placed in every skill directory skrills writes, so a later cleanup
/// can tell its copies from skills the user made. Hidden, so no reader
/// collects it as a module.
pub(crate) const SYNCED_MARKER: &str = ".skrills-synced";

/// Writes skill-shaped directories (`<root>/<rel_dir>/<main_file>` plus
/// companion modules) for one batch, with the checks every adapter needs.
///
/// Each adapter used to carry its own copy of this loop, and the copies had
/// drifted: none refused an empty sanitized name or a symlinked destination,
/// two distinct names that sanitized to one directory silently overwrote each
/// other, and three of them skipped the module files whenever `SKILL.md` was
/// unchanged, so an edited companion script never reached the target.
pub(crate) struct BatchWriter {
    root: std::path::PathBuf,
    /// Destination, case-folded, to the item that claimed it in this batch.
    /// Folded because APFS and NTFS treat `Foo` and `foo` as one directory.
    claimed: std::collections::HashMap<String, String>,
}

/// Records the failure of one item's write as a refusal so the rest of the
/// batch is still attempted. Errors that make the whole phase pointless (the
/// target root itself) are raised by the callers before the batch starts.
fn refuse_unwritable(
    item: &str,
    path: &Path,
    error: &std::io::Error,
    report: &mut crate::report::WriteReport,
) {
    report.skipped.push(crate::report::SkipReason::Refused {
        item: item.to_string(),
        reason: format!("could not write {}: {error}", path.display()),
    });
}

impl BatchWriter {
    pub(crate) fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            root: root.into(),
            claimed: std::collections::HashMap::new(),
        }
    }

    /// Writes a single-file item (a command, agent, rule) to
    /// `<root>/<stem><suffix>` with the same refusals as [`Self::write`].
    ///
    /// `stem` must already be sanitized by the caller.
    pub(crate) fn write_single(
        &mut self,
        item: &str,
        stem: &str,
        suffix: &str,
        content: &[u8],
        report: &mut crate::report::WriteReport,
    ) -> std::io::Result<()> {
        use crate::report::SkipReason;

        if stem.is_empty() {
            report.skipped.push(SkipReason::Refused {
                item: item.to_string(),
                reason: "the name sanitizes to an empty file name".to_string(),
            });
            return Ok(());
        }
        let path = self.root.join(format!("{stem}{suffix}"));
        let key = path.to_string_lossy().to_lowercase();
        if let Some(first) = self.claimed.get(&key) {
            report.skipped.push(SkipReason::Refused {
                item: item.to_string(),
                reason: format!(
                    "it maps to the same file as `{first}` ({}); the first one was kept",
                    path.display()
                ),
            });
            return Ok(());
        }
        self.claimed.insert(key, item.to_string());

        if let Some(link) = symlink_below(&self.root, &path) {
            report.skipped.push(SkipReason::Refused {
                item: item.to_string(),
                reason: format!(
                    "{} is a symlink; writing through it would change the file it points to",
                    link.display()
                ),
            });
            return Ok(());
        }
        if std::fs::read(&path).ok().as_deref() == Some(content) {
            report.skipped.push(SkipReason::Unchanged {
                item: item.to_string(),
            });
            return Ok(());
        }
        let written = match path.parent() {
            Some(parent) => create_dir_all(parent),
            None => Ok(()),
        }
        .and_then(|()| write_file(&path, content));
        match written {
            Ok(()) => report.written += 1,
            Err(e) => refuse_unwritable(item, &path, &e, report),
        }
        Ok(())
    }

    /// Writes one item; refusals and unchanged items land in `report`.
    ///
    /// `rel_dir` must already be sanitized by the caller.
    pub(crate) fn write(
        &mut self,
        item: &str,
        rel_dir: &str,
        main_file: &str,
        main: &[u8],
        modules: &[ModuleFile],
        report: &mut crate::report::WriteReport,
    ) -> std::io::Result<()> {
        use crate::report::SkipReason;

        if rel_dir.is_empty() {
            report.skipped.push(SkipReason::Refused {
                item: item.to_string(),
                reason: "the name sanitizes to an empty directory name".to_string(),
            });
            return Ok(());
        }

        let dir = self.root.join(rel_dir);
        let key = dir.to_string_lossy().to_lowercase();
        if let Some(first) = self.claimed.get(&key) {
            report.skipped.push(SkipReason::Refused {
                item: item.to_string(),
                reason: format!(
                    "it maps to the same directory as `{first}` ({}); the first one was kept",
                    dir.display()
                ),
            });
            return Ok(());
        }
        self.claimed.insert(key, item.to_string());

        let main_path = dir.join(main_file);
        if let Some(link) = symlink_below(&self.root, &main_path) {
            report.skipped.push(SkipReason::Refused {
                item: item.to_string(),
                reason: format!(
                    "{} is a symlink; writing through it would change the file it points to",
                    link.display()
                ),
            });
            return Ok(());
        }

        let mut changed = false;
        if std::fs::read(&main_path).ok().as_deref() != Some(main) {
            if let Err(e) = create_dir_all(&dir).and_then(|()| write_file(&main_path, main)) {
                refuse_unwritable(item, &main_path, &e, report);
                return Ok(());
            }
            changed = true;
        }

        for module in modules {
            let module_path = dir.join(&module.relative_path);
            if !is_path_contained(&module_path, &dir) {
                debug!(
                    path = %module.relative_path.display(),
                    "Skipping module with path outside skill directory"
                );
                continue;
            }
            if let Some(link) = symlink_below(&dir, &module_path) {
                report.warnings.push(format!(
                    "Skipped module {} of {item}: {} is a symlink",
                    module.relative_path.display(),
                    link.display()
                ));
                continue;
            }
            if std::fs::read(&module_path).ok().as_deref() == Some(module.content.as_slice()) {
                continue;
            }
            let written = match module_path.parent() {
                Some(parent) => create_dir_all(parent),
                None => Ok(()),
            }
            .and_then(|()| write_file(&module_path, &module.content));
            if let Err(e) = written {
                report.warnings.push(format!(
                    "Skipped module {} of {item}: could not write {}: {e}",
                    module.relative_path.display(),
                    module_path.display()
                ));
                continue;
            }
            changed = true;
        }

        let marker = dir.join(SYNCED_MARKER);
        if !marker.exists() {
            if let Err(e) = write_file(&marker, b"written by skrills sync\n") {
                report.warnings.push(format!(
                    "Could not mark {} as a skrills copy: {e}",
                    dir.display()
                ));
            }
        }

        if changed {
            report.written += 1;
        } else {
            report.skipped.push(SkipReason::Unchanged {
                item: item.to_string(),
            });
        }
        Ok(())
    }
}

/// Whether `file` sits inside a skill directory below `root`: some directory
/// between them (excluding `root`) holds a `SKILL.md`. Such a markdown file is
/// one of that skill's modules, not a legacy single-file skill.
pub(crate) fn inside_skill_dir(file: &Path, root: &Path) -> bool {
    file.ancestors()
        .skip(1)
        .take_while(|dir| *dir != root && dir.starts_with(root))
        .any(|dir| dir.join("SKILL.md").is_file())
}

/// Test helper functions for adapter tests.
#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::common::Command;
    use std::path::PathBuf;
    use std::time::SystemTime;

    /// Create a Command with minimal boilerplate. Uses deterministic defaults
    /// for source_path, modified, and hash so tests are reproducible.
    pub fn make_command(name: &str, content: &str) -> Command {
        let mut cmd = Command::new(
            name.to_string(),
            content.as_bytes().to_vec(),
            PathBuf::from(format!("/test/{name}.md")),
        );
        cmd.modified = SystemTime::UNIX_EPOCH;
        cmd
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hidden_component() {
        assert!(is_hidden_component(".git"));
        assert!(is_hidden_component(".hidden"));
        assert!(!is_hidden_component("visible"));
        assert!(!is_hidden_component(""));
    }

    #[test]
    fn test_is_hidden_path() {
        assert!(is_hidden_path(Path::new(".git/config")));
        assert!(is_hidden_path(Path::new("foo/.hidden/bar")));
        assert!(!is_hidden_path(Path::new("foo/bar/baz")));
        assert!(!is_hidden_path(Path::new("visible.txt")));
    }

    #[test]
    fn test_hash_content() {
        let hash = hash_content(b"hello");
        assert_eq!(hash.len(), 64); // SHA-256 produces 64 hex chars
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// Sync compares these strings to decide a file is unchanged, so the
    /// encoding has to survive a `sha2`/`digest` upgrade: lowercase, two
    /// digits per byte, FIPS 180-2 vector for "abc".
    #[test]
    fn hash_content_matches_known_sha256_vector() {
        assert_eq!(
            hash_content(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sanitize_name_removes_path_traversal() {
        assert_eq!(sanitize_name("../../../etc/passwd"), "etcpasswd");
        assert_eq!(sanitize_name("valid-name_123"), "valid-name_123");
        assert_eq!(sanitize_name("../../malicious"), "malicious");
        assert_eq!(sanitize_name("normal"), "normal");
        assert_eq!(sanitize_name("with spaces"), "withspaces");
        assert_eq!(sanitize_name("has/slashes"), "hasslashes");
    }

    #[test]
    fn sanitize_name_segments_handles_traversal() {
        assert_eq!(sanitize_name_segments("../../../etc/passwd"), "etc/passwd");
        assert_eq!(sanitize_name_segments("foo/../bar"), "foo/bar");
        assert_eq!(sanitize_name_segments("foo/./bar"), "foo/bar");
        assert_eq!(sanitize_name_segments(".."), "");
        assert_eq!(sanitize_name_segments("."), "");
        assert_eq!(
            sanitize_name_segments("category/my-skill"),
            "category/my-skill"
        );
        assert_eq!(sanitize_name_segments("foo///bar"), "foo/bar");
        assert_eq!(sanitize_name_segments("my skill!@#"), "myskill");
    }

    #[test]
    fn sanitize_name_kebab_converts_to_kebab_case() {
        assert_eq!(sanitize_name_kebab("My Skill Name"), "my-skill-name");
        assert_eq!(
            sanitize_name_kebab("skill_with_underscores"),
            "skill-with-underscores"
        );
        assert_eq!(sanitize_name_kebab("Already-Kebab"), "already-kebab");
        assert_eq!(sanitize_name_kebab("file.name.ext"), "file-name-ext");
        assert_eq!(sanitize_name_kebab("CLAUDE.md"), "claude-md");
        // Slashes are stripped (not alphanumeric, not in map)
        assert_eq!(sanitize_name_kebab("../../../etc/passwd"), "etcpasswd");
    }

    #[test]
    fn atomic_write_replaces_content_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"old").unwrap();

        atomic_write(&path, b"new", false).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.json".to_string()]);
    }

    /// A crash between truncate and write used to leave the user's config
    /// empty. The rename means the destination is never opened for writing,
    /// so a write that fails part way leaves the old content in place.
    #[cfg(unix)]
    #[test]
    fn atomic_write_failure_leaves_the_original_intact() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, b"original").unwrap();
        // A read-only directory refuses the temp file, so the write fails.
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();

        let result = atomic_write(&path, b"replacement", false);

        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        if result.is_ok() {
            // Running as root ignores directory permissions; nothing to assert.
            return;
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_a_symlinked_destination_a_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("dotfiles-settings.json");
        let link = dir.path().join("settings.json");
        std::fs::write(&real, b"old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();

        atomic_write(&link, b"new", false).unwrap();

        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&real).unwrap(), b"new");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_existing_mode_and_makes_new_private_files_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("settings.json");
        std::fs::write(&existing, b"{}").unwrap();
        std::fs::set_permissions(&existing, std::fs::Permissions::from_mode(0o640)).unwrap();

        atomic_write(&existing, b"{\"a\":1}", true).unwrap();
        let fresh = dir.path().join("mcp.json");
        atomic_write(&fresh, b"{}", true).unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&existing), 0o640);
        assert_eq!(mode(&fresh), 0o600);
    }

    /// A recorded session for the helper tests below, returning the entries
    /// the recorder stored once the session ends.
    fn recorded_by(tmp: &tempfile::TempDir, change: impl FnOnce()) -> Vec<crate::snapshot::Entry> {
        let dir = tmp.path().join("snapshot");
        let session = crate::write_session::WriteSession::install(&dir).unwrap();
        change();
        drop(session);
        if dir.exists() {
            crate::snapshot::read_manifest(&dir).unwrap()
        } else {
            Vec::new()
        }
    }

    #[test]
    fn atomic_write_records_the_old_bytes_before_replacing_them() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("cfg.json");
        std::fs::write(&path, b"old").unwrap();
        let entries = recorded_by(&tmp, || atomic_write(&path, b"new", false).unwrap());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, path);
        assert_eq!(
            entries[0].prior_hash.as_deref(),
            Some(hash_content(b"old").as_str())
        );
    }

    #[test]
    fn atomic_write_through_a_symlink_records_the_target_it_replaces() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.md");
        let link = tmp.path().join("link.md");
        std::fs::write(&real, b"old").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let entries = recorded_by(&tmp, || atomic_write(&link, b"new", false).unwrap());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, std::fs::canonicalize(&real).unwrap());
    }

    #[test]
    fn atomic_write_records_the_parent_directories_it_creates() {
        let tmp = tempfile::tempdir().unwrap();
        let new_dir = tmp.path().join("new");
        let path = new_dir.join("sub").join("f.md");
        let entries = recorded_by(&tmp, || atomic_write(&path, b"x", false).unwrap());
        let dirs: Vec<_> = entries
            .iter()
            .filter(|e| e.kind == crate::snapshot::Kind::Dir)
            .map(|e| e.path.clone())
            .collect();
        assert_eq!(dirs, [new_dir.clone(), new_dir.join("sub")]);
        assert!(entries.iter().any(|e| e.path == path));
    }

    #[test]
    fn create_dir_all_records_new_directories_then_creates_them() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("one").join("two");
        let entries = recorded_by(&tmp, || {
            create_dir_all(&deep).unwrap();
            write_file(deep.join("f"), b"x").unwrap();
        });
        assert!(deep.is_dir());
        let dirs: Vec<_> = entries
            .iter()
            .filter(|e| e.kind == crate::snapshot::Kind::Dir)
            .map(|e| e.path.clone())
            .collect();
        assert_eq!(dirs, [tmp.path().join("one"), deep.clone()]);
    }

    /// `first` -> `second` -> missing: the write used to stop at `second`
    /// and rename over it, breaking the chain. It now follows the chain as
    /// `copy` does and creates the missing file at its end.
    #[cfg(unix)]
    #[test]
    fn atomic_write_through_a_chain_ending_in_a_missing_file_keeps_every_link() {
        let tmp = tempfile::tempdir().unwrap();
        let first = tmp.path().join("first");
        let second = tmp.path().join("second");
        let missing = tmp.path().join("missing");
        std::os::unix::fs::symlink(&second, &first).unwrap();
        std::os::unix::fs::symlink(&missing, &second).unwrap();
        let entries = recorded_by(&tmp, || atomic_write(&first, b"new", false).unwrap());
        assert!(std::fs::symlink_metadata(&first).unwrap().is_symlink());
        assert!(std::fs::symlink_metadata(&second).unwrap().is_symlink());
        assert_eq!(std::fs::read(&missing).unwrap(), b"new");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, missing);
        assert_eq!(entries[0].prior, crate::snapshot::Prior::Absent);
    }

    /// A loop has no end to write to; renaming over one of its links would
    /// replace the user's link with a file.
    #[cfg(unix)]
    #[test]
    fn atomic_write_and_copy_into_a_symlink_loop_fail_and_keep_the_links() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b, src) = (
            tmp.path().join("a"),
            tmp.path().join("b"),
            tmp.path().join("src"),
        );
        std::os::unix::fs::symlink(&b, &a).unwrap();
        std::os::unix::fs::symlink(&a, &b).unwrap();
        std::fs::write(&src, b"x").unwrap();

        assert!(atomic_write(&a, b"new", false).is_err());
        assert!(copy(&src, &a).is_err());

        assert!(std::fs::symlink_metadata(&a).unwrap().is_symlink());
        assert!(std::fs::symlink_metadata(&b).unwrap().is_symlink());
    }

    #[cfg(unix)]
    #[test]
    fn copy_onto_a_symlink_records_the_target_it_writes_through() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, real, link) = (
            tmp.path().join("src"),
            tmp.path().join("real"),
            tmp.path().join("link"),
        );
        std::fs::write(&src, b"from source").unwrap();
        std::fs::write(&real, b"was here").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let entries = recorded_by(&tmp, || {
            copy(&src, &link).unwrap();
        });
        assert!(std::fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(std::fs::read(&real).unwrap(), b"from source");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, std::fs::canonicalize(&real).unwrap());
        assert_eq!(
            entries[0].prior_hash.as_deref(),
            Some(hash_content(b"was here").as_str())
        );
    }

    #[cfg(unix)]
    #[test]
    fn remove_tree_of_a_symlinked_directory_records_the_link_not_its_contents() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        let link = tmp.path().join("link");
        std::fs::create_dir(&real).unwrap();
        std::fs::write(real.join("keep.md"), b"k").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let entries = recorded_by(&tmp, || remove_tree(&link).unwrap());
        assert!(real.join("keep.md").exists());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, link);
        assert_eq!(entries[0].kind, crate::snapshot::Kind::Symlink);
    }

    #[test]
    fn write_config_records_the_config_before_its_backup_and_both_after_hashes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        let backup = tmp.path().join("CLAUDE.md.skrills-bak");
        std::fs::write(&path, b"mine").unwrap();
        let entries = recorded_by(&tmp, || {
            assert!(write_config(&path, b"synced", false).unwrap());
        });
        let paths: Vec<_> = entries.iter().map(|e| e.path.clone()).collect();
        assert_eq!(paths, [path.clone(), backup.clone()]);
        assert_eq!(entries[1].prior, crate::snapshot::Prior::Absent);
        assert_eq!(
            entries[0].after,
            Some(crate::snapshot::After::File(hash_content(b"synced")))
        );
        assert_eq!(
            entries[1].after,
            Some(crate::snapshot::After::File(hash_content(b"mine")))
        );
    }

    #[test]
    fn atomic_write_records_the_after_hash_of_what_it_wrote() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f");
        let entries = recorded_by(&tmp, || atomic_write(&path, b"new", false).unwrap());
        assert_eq!(
            entries[0].after,
            Some(crate::snapshot::After::File(hash_content(b"new")))
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_atomic_write_records_no_after_hash() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let locked = tmp.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        let path = locked.join("f");
        std::fs::write(&path, b"old").unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        let mut outcome = Ok(());
        let entries = recorded_by(&tmp, || outcome = atomic_write(&path, b"new", false));
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        // A privileged user can write anyway; then the after hash is right.
        match outcome {
            Ok(()) => assert!(entries[0].after.is_some()),
            Err(_) => assert_eq!(entries[0].after, None),
        }
    }

    #[test]
    fn copy_records_the_after_hash_of_the_copied_content() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dest) = (tmp.path().join("src"), tmp.path().join("dest"));
        std::fs::write(&src, b"from source").unwrap();
        let entries = recorded_by(&tmp, || {
            copy(&src, &dest).unwrap();
        });
        assert_eq!(
            entries[0].after,
            Some(crate::snapshot::After::File(hash_content(b"from source")))
        );
    }

    #[test]
    fn remove_file_records_the_path_as_absent_after_the_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gone");
        std::fs::write(&path, b"x").unwrap();
        let entries = recorded_by(&tmp, || remove_file(&path).unwrap());
        assert_eq!(entries[0].after, Some(crate::snapshot::After::Absent));
    }

    #[test]
    fn remove_tree_records_each_removed_file_as_absent_after_the_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let tree = tmp.path().join("tree");
        std::fs::create_dir_all(tree.join("sub")).unwrap();
        std::fs::write(tree.join("a"), b"1").unwrap();
        std::fs::write(tree.join("sub/b"), b"2").unwrap();
        let entries = recorded_by(&tmp, || remove_tree(&tree).unwrap());
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .all(|e| e.after == Some(crate::snapshot::After::Absent)));
    }

    #[test]
    fn write_config_with_unchanged_content_records_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("CLAUDE.md");
        std::fs::write(&path, b"same").unwrap();
        let entries = recorded_by(&tmp, || {
            assert!(!write_config(&path, b"same", false).unwrap());
        });
        assert!(entries.is_empty());
    }

    #[test]
    fn remove_file_records_the_file_then_deletes_it() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("gone.md");
        std::fs::write(&path, b"keep me").unwrap();
        let entries = recorded_by(&tmp, || remove_file(&path).unwrap());
        assert!(!path.exists());
        assert_eq!(
            entries[0].prior_hash.as_deref(),
            Some(hash_content(b"keep me").as_str())
        );
    }

    #[test]
    fn remove_file_of_a_missing_path_reports_the_os_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = remove_file(&tmp.path().join("nope")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn remove_tree_records_every_file_in_the_tree_then_deletes_it() {
        let tmp = tempfile::tempdir().unwrap();
        let tree = tmp.path().join("tree");
        std::fs::create_dir_all(tree.join("a/b")).unwrap();
        std::fs::write(tree.join("top"), b"1").unwrap();
        std::fs::write(tree.join("a/b/deep"), b"2").unwrap();
        let entries = recorded_by(&tmp, || remove_tree(&tree).unwrap());
        assert!(!tree.exists());
        let mut paths: Vec<_> = entries.iter().map(|e| e.path.clone()).collect();
        paths.sort();
        assert_eq!(paths, [tree.join("a/b/deep"), tree.join("top")]);
    }

    #[test]
    fn remove_tree_of_a_missing_path_reports_the_os_error() {
        let tmp = tempfile::tempdir().unwrap();
        let err = remove_tree(&tmp.path().join("nope")).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn remove_dir_removes_an_empty_directory_and_refuses_a_full_one() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty");
        let full = tmp.path().join("full");
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&full).unwrap();
        std::fs::write(full.join("f"), b"x").unwrap();
        let entries = recorded_by(&tmp, || {
            remove_dir(&empty).unwrap();
            assert!(remove_dir(&full).is_err());
        });
        assert!(!empty.exists());
        assert!(full.join("f").exists());
        assert!(entries.is_empty());
    }

    #[test]
    fn copy_records_the_destinations_old_bytes_and_copies_the_source() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dest) = (tmp.path().join("src"), tmp.path().join("dest"));
        std::fs::write(&src, b"from source").unwrap();
        std::fs::write(&dest, b"was here").unwrap();
        let entries = recorded_by(&tmp, || {
            assert_eq!(copy(&src, &dest).unwrap(), 11);
        });
        assert_eq!(std::fs::read(&dest).unwrap(), b"from source");
        assert_eq!(entries[0].path, dest);
        assert_eq!(
            entries[0].prior_hash.as_deref(),
            Some(hash_content(b"was here").as_str())
        );
    }

    #[test]
    fn helpers_outside_a_session_just_do_the_work() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f");
        atomic_write(&path, b"x", false).unwrap();
        remove_file(&path).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn write_config_backs_up_the_previous_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("CLAUDE.md");
        std::fs::write(&path, b"mine").unwrap();

        assert!(write_config(&path, b"synced", false).unwrap());

        assert_eq!(std::fs::read(&path).unwrap(), b"synced");
        assert_eq!(
            std::fs::read(dir.path().join("CLAUDE.md.skrills-bak")).unwrap(),
            b"mine"
        );
    }

    #[test]
    fn write_config_skips_unchanged_content_and_takes_no_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, b"same").unwrap();

        assert!(!write_config(&path, b"same", false).unwrap());
        assert!(!dir.path().join("settings.json.skrills-bak").exists());
    }

    #[test]
    fn collect_module_files_from_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let modules = collect_module_files(dir.path());
        assert!(modules.is_empty());
    }

    #[test]
    fn collect_module_files_skips_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SKILL.md"), "main skill").unwrap();
        std::fs::write(dir.path().join("helper.py"), "# helper").unwrap();
        let modules = collect_module_files(dir.path());
        assert_eq!(modules.len(), 1);
        assert_eq!(
            modules[0].relative_path,
            std::path::PathBuf::from("helper.py")
        );
    }

    #[test]
    fn inside_skill_dir_stops_at_the_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("skills");
        std::fs::create_dir_all(root.join("foo/docs")).unwrap();
        std::fs::write(root.join("foo/SKILL.md"), "x").unwrap();
        std::fs::write(root.join("SKILL.md"), "stray").unwrap();

        assert!(inside_skill_dir(&root.join("foo/docs/ref.md"), &root));
        assert!(!inside_skill_dir(&root.join("legacy.md"), &root));
    }

    /// A symlink inside a skill points anywhere (`~/.ssh/id_ed25519`, say);
    /// following it copied the target into every other tool's directory.
    #[cfg(unix)]
    #[test]
    fn collect_module_files_does_not_follow_file_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "key").unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("linked"))
            .unwrap();
        std::fs::write(dir.path().join("plain.py"), "x").unwrap();

        let modules = collect_module_files(dir.path());

        let names: Vec<_> = modules.iter().map(|m| m.relative_path.clone()).collect();
        assert_eq!(names, vec![std::path::PathBuf::from("plain.py")]);
    }

    /// A nested directory with its own SKILL.md is a separate skill: its
    /// SKILL.md was dropped but its other files kept, leaving a headless
    /// `sub/` inside the parent skill.
    #[test]
    fn collect_module_files_leaves_out_a_nested_skill() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SKILL.md"), "root").unwrap();
        std::fs::write(dir.path().join("x.py"), "x").unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/SKILL.md"), "sub").unwrap();
        std::fs::write(dir.path().join("sub/y.py"), "y").unwrap();
        std::fs::create_dir_all(dir.path().join("lib")).unwrap();
        std::fs::write(dir.path().join("lib/z.py"), "z").unwrap();

        let mut names: Vec<_> = collect_module_files(dir.path())
            .into_iter()
            .map(|m| m.relative_path)
            .collect();
        names.sort();

        assert_eq!(
            names,
            vec![
                std::path::PathBuf::from("lib/z.py"),
                std::path::PathBuf::from("x.py")
            ]
        );
    }

    /// Every companion file was read whole into memory with no cap.
    #[test]
    fn collect_module_files_skips_files_over_the_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        let big = std::fs::File::create(dir.path().join("big.bin")).unwrap();
        big.set_len(MAX_MODULE_FILE_BYTES + 1).unwrap();
        std::fs::write(dir.path().join("small.py"), "x").unwrap();

        let modules = collect_module_files(dir.path());

        let names: Vec<_> = modules.iter().map(|m| m.relative_path.clone()).collect();
        assert_eq!(names, vec![std::path::PathBuf::from("small.py")]);
    }

    /// `canonicalize` fails on a link whose target does not exist yet, and the
    /// fallback replaced the link itself with a plain file.
    #[cfg(unix)]
    #[test]
    fn atomic_write_keeps_a_dangling_symlink_and_creates_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("settings.json");
        std::os::unix::fs::symlink("dotfiles/settings.json", &link).unwrap();
        std::fs::create_dir_all(dir.path().join("dotfiles")).unwrap();

        atomic_write(&link, b"{}", false).unwrap();

        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(
            std::fs::read(dir.path().join("dotfiles/settings.json")).unwrap(),
            b"{}"
        );
    }

    fn refused_items(report: &crate::report::WriteReport) -> Vec<String> {
        report
            .skipped
            .iter()
            .filter_map(|skip| match skip {
                crate::report::SkipReason::Refused { item, .. } => Some(item.clone()),
                _ => None,
            })
            .collect()
    }

    /// One item that cannot be written used to abort the batch through `?`,
    /// leaving every later item unwritten on this run and on every run after.
    #[test]
    fn batch_writer_refuses_an_item_it_cannot_write_and_writes_the_rest() {
        let root = tempfile::tempdir().unwrap();
        // A plain file where the item's directory has to go.
        std::fs::write(root.path().join("blocked"), b"in the way").unwrap();
        let mut writer = BatchWriter::new(root.path());
        let mut report = crate::report::WriteReport::default();

        writer
            .write("blocked", "blocked", "SKILL.md", b"a", &[], &mut report)
            .unwrap();
        writer
            .write("fine", "fine", "SKILL.md", b"b", &[], &mut report)
            .unwrap();

        assert_eq!(report.written, 1);
        assert!(root.path().join("fine/SKILL.md").exists());
        assert_eq!(refused_items(&report), vec!["blocked".to_string()]);
    }

    #[test]
    fn batch_writer_single_refuses_an_item_it_cannot_write_and_writes_the_rest() {
        let root = tempfile::tempdir().unwrap();
        // A directory where the item's file has to go.
        std::fs::create_dir_all(root.path().join("blocked.md/inner")).unwrap();
        let mut writer = BatchWriter::new(root.path());
        let mut report = crate::report::WriteReport::default();

        writer
            .write_single("blocked", "blocked", ".md", b"a", &mut report)
            .unwrap();
        writer
            .write_single("fine", "fine", ".md", b"b", &mut report)
            .unwrap();

        assert_eq!(report.written, 1);
        assert!(root.path().join("fine.md").is_file());
        assert_eq!(refused_items(&report), vec!["blocked".to_string()]);
    }

    /// APFS and NTFS are case-insensitive by default, so `Foo` and `foo` are
    /// one directory there and the second write replaced the first.
    #[test]
    fn batch_writer_refuses_a_name_that_differs_only_in_case() {
        let root = tempfile::tempdir().unwrap();
        let mut writer = BatchWriter::new(root.path());
        let mut report = crate::report::WriteReport::default();

        writer
            .write("Foo", "Foo", "SKILL.md", b"first", &[], &mut report)
            .unwrap();
        writer
            .write("foo", "foo", "SKILL.md", b"second", &[], &mut report)
            .unwrap();
        writer
            .write_single("Bar", "Bar", ".md", b"first", &mut report)
            .unwrap();
        writer
            .write_single("bar", "bar", ".md", b"second", &mut report)
            .unwrap();

        assert_eq!(report.written, 2);
        assert_eq!(
            std::fs::read(root.path().join("Foo/SKILL.md")).unwrap(),
            b"first"
        );
        assert_eq!(
            refused_items(&report),
            vec!["foo".to_string(), "bar".to_string()]
        );
    }

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn sanitize_name_never_contains_path_traversal(s in "\\PC*") {
                let sanitized = sanitize_name(&s);
                prop_assert!(!sanitized.contains(".."), "sanitized name contained ..: {}", sanitized);
                prop_assert!(!sanitized.contains('/'), "sanitized name contained /: {}", sanitized);
                prop_assert!(!sanitized.contains('\\'), "sanitized name contained \\: {}", sanitized);
            }

            #[test]
            fn sanitize_name_segments_never_contains_traversal(s in "\\PC*") {
                let sanitized = sanitize_name_segments(&s);
                // No segment should be ".." after sanitization
                for segment in sanitized.split('/') {
                    prop_assert!(segment != "..", "segment was ..: {}", sanitized);
                    prop_assert!(segment != ".", "segment was .: {}", sanitized);
                }
                // Should never start with "/" (no absolute paths from arbitrary input)
                prop_assert!(!sanitized.starts_with('/'), "produced absolute path: {}", sanitized);
            }

            #[test]
            fn sanitize_name_only_contains_safe_chars(s in "\\PC*") {
                let sanitized = sanitize_name(&s);
                for c in sanitized.chars() {
                    prop_assert!(
                        c.is_alphanumeric() || c == '-' || c == '_',
                        "unexpected char {:?} in sanitized name: {}", c, sanitized
                    );
                }
            }

            #[test]
            fn sanitize_name_kebab_never_contains_path_separators(s in "\\PC*") {
                let sanitized = sanitize_name_kebab(&s);
                prop_assert!(!sanitized.contains('/'), "kebab name contained /: {}", sanitized);
                prop_assert!(!sanitized.contains('\\'), "kebab name contained \\: {}", sanitized);
                prop_assert!(!sanitized.contains(".."), "kebab name contained ..: {}", sanitized);
                // ASCII characters should be lowercased (function uses to_ascii_lowercase)
                for c in sanitized.chars() {
                    if c.is_ascii() {
                        prop_assert!(
                            !c.is_ascii_uppercase(),
                            "ASCII char {:?} was not lowercased in: {}", c, sanitized
                        );
                    }
                }
                // No consecutive hyphens
                prop_assert!(
                    !sanitized.contains("--"),
                    "kebab name contained consecutive hyphens: {}", sanitized
                );
            }

            #[test]
            fn hash_content_never_panics(data in proptest::collection::vec(any::<u8>(), 0..1024)) {
                let hash = hash_content(&data);
                prop_assert_eq!(hash.len(), 64, "hash was not 64 hex chars");
                prop_assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "hash contained non-hex chars");
            }
        }
    }
}
