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
/// On engagement, returns [`SyncError::TokenBudgetExceeded`] wrapped in
/// `anyhow::Error`. The numeric `tokens` and `ceiling` fields are not
/// available at the adapter (they live in the engine that engaged the
/// switch); the adapter reports `(0, 0)` and relies on the engine's
/// WARNING-tier alert for the operator-facing numbers.
pub(crate) fn ensure_not_engaged(switch: Option<&KillSwitch>) -> Result<()> {
    if let Some(s) = switch {
        if s.is_engaged() {
            return Err(SyncError::TokenBudgetExceeded {
                tokens: 0,
                ceiling: 0,
            }
            .into());
        }
    }
    Ok(())
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

    for entry in WalkDir::new(skill_dir)
        .min_depth(1)
        .max_depth(10)
        .follow_links(false)
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

        if path.file_name().is_some_and(|n| n == "SKILL.md") {
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
pub(crate) fn atomic_write(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<()> {
    use std::io::Write;

    let dest = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    };
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
pub(crate) fn write_file(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> std::io::Result<()> {
    atomic_write(path.as_ref(), bytes.as_ref(), false)
}

/// Writes a user-owned config file (`settings.json`, `CLAUDE.md`, `mcp.json`,
/// `config.toml`, ...): no-op when the content is unchanged, otherwise keeps a
/// one-generation copy of the previous content at `<path>.skrills-bak` and
/// replaces the file atomically.
///
/// Returns whether the file was written. Backups are only kept for config
/// files: a `.skrills-bak` beside a skill would be picked up as one of its
/// module files on the next sync.
pub(crate) fn write_config(path: &Path, bytes: &[u8], private: bool) -> std::io::Result<bool> {
    match std::fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(false),
        Ok(existing) => {
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
    /// Destination directory to the item that claimed it in this batch.
    claimed: std::collections::HashMap<std::path::PathBuf, String>,
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
        if let Some(first) = self.claimed.get(&path) {
            report.skipped.push(SkipReason::Refused {
                item: item.to_string(),
                reason: format!(
                    "it maps to the same file as `{first}` ({}); the first one was kept",
                    path.display()
                ),
            });
            return Ok(());
        }
        self.claimed.insert(path.clone(), item.to_string());

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
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_file(&path, content)?;
        report.written += 1;
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
        if let Some(first) = self.claimed.get(&dir) {
            report.skipped.push(SkipReason::Refused {
                item: item.to_string(),
                reason: format!(
                    "it maps to the same directory as `{first}` ({}); the first one was kept",
                    dir.display()
                ),
            });
            return Ok(());
        }
        self.claimed.insert(dir.clone(), item.to_string());

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
            std::fs::create_dir_all(&dir)?;
            write_file(&main_path, main)?;
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
            if let Some(parent) = module_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            write_file(&module_path, &module.content)?;
            changed = true;
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
