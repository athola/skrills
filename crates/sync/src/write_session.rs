//! Records what a sync is about to overwrite or delete, so
//! `skrills sync-rollback` can put it back.
//!
//! A [`WriteSession`] is a thread-local guard. While one is installed, the
//! write, copy and removal helpers in `adapters/utils.rs` report each path
//! they are about to change. The first report for a path stores its prior
//! state (bytes and mode, or "absent") in the session's directory and appends
//! one line to its manifest, fsynced, so an interrupt mid-sync still leaves a
//! manifest that matches what was changed. The recorder only observes: it
//! never skips or alters a write. A record it cannot store fails that write,
//! because a change that cannot be undone is worse than a refused one.
//!
//! The directory is `0700` and its files `0600`; configs can hold secrets.

use crate::adapters::utils::{atomic_write_unrecorded, hash_content, remove_tree_unrecorded};
use crate::snapshot::{After, AfterLine, Entry, Kind, Prior, BLOB_DIR, MANIFEST_FILE};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::Write;
use std::marker::PhantomData;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

thread_local! {
    static ACTIVE: RefCell<Option<Recorder>> = const { RefCell::new(None) };
}

struct Recorder {
    dir: PathBuf,
    manifest: std::fs::File,
    seen: HashSet<PathBuf>,
    /// Whether anything other than a created directory was recorded.
    changed_content: bool,
}

/// Keeps a recorder installed on this thread until dropped.
///
/// A session whose sync changed nothing removes its directory on drop, so an
/// idle sync does not bury the last useful snapshot under an empty one. A
/// sync that only created empty directories (adapters make their target
/// folders up front) counts as idle: those directories are left behind, as
/// they were before directories were recorded.
/// Not `Send`: the recorder lives in the installing thread's storage.
#[must_use = "dropping the session immediately stops recording"]
pub struct WriteSession {
    _same_thread: PhantomData<*const ()>,
}

/// Whether a session is installed on this thread.
pub fn is_active() -> bool {
    ACTIVE.with(|active| active.borrow().is_some())
}

impl WriteSession {
    /// Creates `dir` (which must not exist yet) and starts recording into it.
    ///
    /// # Panics
    ///
    /// Panics when a session is already installed on this thread. Sessions do
    /// not nest silently: a caller that may run inside another session checks
    /// [`is_active`] and joins it.
    pub fn install(dir: &Path) -> std::io::Result<Self> {
        assert!(
            !is_active(),
            "a write session is already installed on this thread; nesting must be explicit"
        );
        create_private_dir(dir)?;
        let manifest = open_manifest(&dir.join(MANIFEST_FILE))?;
        ACTIVE.with(|active| {
            *active.borrow_mut() = Some(Recorder {
                dir: dir.to_path_buf(),
                manifest,
                seen: HashSet::new(),
                changed_content: false,
            });
        });
        Ok(Self {
            _same_thread: PhantomData,
        })
    }
}

impl Drop for WriteSession {
    fn drop(&mut self) {
        let Some(recorder) = ACTIVE.with(|active| active.borrow_mut().take()) else {
            return;
        };
        if recorder.changed_content {
            return;
        }
        if let Err(e) = remove_tree_unrecorded(&recorder.dir) {
            tracing::warn!(
                path = %recorder.dir.display(),
                error = %e,
                "Could not remove an empty snapshot directory"
            );
        }
    }
}

pub(crate) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::DirBuilder::new().create(dir)
    }
}

fn open_manifest(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Stores `path`'s prior state before a write or removal changes it. Does
/// nothing outside a session.
pub(crate) fn record_before_write(path: &Path) -> std::io::Result<()> {
    ACTIVE.with(|active| match active.borrow_mut().as_mut() {
        Some(recorder) => recorder.record(path),
        None => Ok(()),
    })
}

/// Appends the hash of `bytes`, which a write just put at `path`, so a
/// rollback can tell whether anyone changed the file since. Call it only once
/// the write succeeded. Does nothing outside a session or for a path this
/// session never recorded.
pub(crate) fn record_after_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    record_after(path, || Ok(After::File(hash_content(bytes))))
}

/// [`record_after_write`] for a copy, whose bytes are read back from `path`.
pub(crate) fn record_after_copy(path: &Path) -> std::io::Result<()> {
    record_after(path, || {
        Ok(After::File(hash_content(&std::fs::read(path)?)))
    })
}

/// Notes that the sync removed `path`, so a write earlier in the same sync is
/// not mistaken for the user's file later.
pub(crate) fn record_after_remove(path: &Path) -> std::io::Result<()> {
    record_after(path, || Ok(After::Absent))
}

fn record_after(
    path: &Path,
    state: impl FnOnce() -> std::io::Result<After>,
) -> std::io::Result<()> {
    ACTIVE.with(|active| match active.borrow_mut().as_mut() {
        Some(recorder) => {
            let path = checked_path(path)?;
            if !recorder.seen.contains(&path) {
                return Ok(());
            }
            let after = AfterLine {
                path,
                after: state()?,
            };
            recorder.write_line(&serde_json::to_vec(&after)?)
        }
        None => Ok(()),
    })
}

/// Stores the prior state of every file under `path` before the tree is
/// removed, and returns the paths it recorded so the removal can mark them
/// absent afterwards. A `path` that is already gone records nothing; the
/// removal reports that itself.
pub(crate) fn record_tree_before_remove(path: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut recorded = Vec::new();
    if !is_active() {
        return Ok(recorded);
    }
    // A root that is a symlink is recorded as the link: removing the tree
    // unlinks it and leaves what it points to alone.
    for entry in WalkDir::new(path)
        .follow_links(false)
        .follow_root_links(false)
        .sort_by_file_name()
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e)
                if e.io_error()
                    .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
            {
                return Ok(recorded);
            }
            Err(e) => return Err(e.into()),
        };
        if !entry.file_type().is_dir() {
            record_before_write(entry.path())?;
            recorded.push(entry.path().to_path_buf());
        }
    }
    Ok(recorded)
}

/// Records each directory missing on the way down to `dir` as one the sync
/// creates, shallowest first, so a rollback can remove what the sync added
/// to the tree and not only the files. Does nothing outside a session.
pub(crate) fn record_dirs_before_create(dir: &Path) -> std::io::Result<()> {
    ACTIVE.with(|active| match active.borrow_mut().as_mut() {
        Some(recorder) => {
            let dir = checked_path(dir)?;
            let mut missing: Vec<&Path> = dir
                .ancestors()
                .take_while(|ancestor| {
                    matches!(std::fs::symlink_metadata(ancestor),
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound)
                })
                .collect();
            missing.reverse();
            for created in missing {
                if recorder.seen.contains(created) {
                    // The sync removed what was here earlier; say what it
                    // leaves instead, or rollback would take this
                    // directory for someone else's.
                    let after = AfterLine {
                        path: created.to_path_buf(),
                        after: After::Dir,
                    };
                    recorder.write_line(&serde_json::to_vec(&after)?)?;
                    continue;
                }
                let entry = Entry {
                    path: created.to_path_buf(),
                    prior: Prior::Absent,
                    prior_hash: None,
                    mode: None,
                    kind: Kind::Dir,
                    link_target: None,
                    after: None,
                };
                recorder.append(entry, None)?;
            }
            Ok(())
        }
        None => Ok(()),
    })
}

/// `path` made absolute, refused when it holds `..`, which would let a
/// manifest name a path outside what the sync wrote.
fn checked_path(path: &Path) -> std::io::Result<PathBuf> {
    let path = std::path::absolute(path)?;
    if path.components().any(|part| part == Component::ParentDir) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("cannot snapshot {}: it contains `..`", path.display()),
        ));
    }
    Ok(path)
}

impl Recorder {
    /// Appends one manifest line and syncs it, so an interrupt leaves a
    /// manifest that matches what was changed.
    fn write_line(&mut self, json: &[u8]) -> std::io::Result<()> {
        let mut line = json.to_vec();
        line.push(b'\n');
        self.manifest.write_all(&line)?;
        self.manifest.sync_all()
    }

    fn record(&mut self, path: &Path) -> std::io::Result<()> {
        let path = checked_path(path)?;
        if path.starts_with(&self.dir) || self.seen.contains(&path) {
            return Ok(());
        }
        let Some((entry, bytes)) = capture(&path)? else {
            return Ok(());
        };
        self.append(entry, bytes)
    }

    /// Stores `entry` and its prior bytes, unless its path is the snapshot's
    /// own or was already recorded in this session.
    fn append(&mut self, entry: Entry, bytes: Option<Vec<u8>>) -> std::io::Result<()> {
        if entry.path.starts_with(&self.dir) || self.seen.contains(&entry.path) {
            return Ok(());
        }
        if let (Some(bytes), Some(hash)) = (bytes, entry.prior_hash.as_deref()) {
            let blob = self.dir.join(BLOB_DIR).join(hash);
            if !blob.exists() {
                if let Some(blobs) = blob.parent() {
                    if !blobs.exists() {
                        create_private_dir(blobs)?;
                    }
                }
                atomic_write_unrecorded(&blob, &bytes, true)?;
            }
        }
        self.write_line(&serde_json::to_vec(&entry)?)?;
        self.changed_content |= entry.kind != Kind::Dir;
        self.seen.insert(entry.path);
        Ok(())
    }
}

/// The manifest entry for `path` and the bytes to keep, or `None` for an
/// existing directory, which holds nothing of its own to restore.
///
/// The kind comes from `symlink_metadata` before anything is read: a symlink
/// is kept as its target and never followed, and a fifo or device is never
/// opened, since reading one can block or consume someone else's data.
fn capture(path: &Path) -> std::io::Result<Option<(Entry, Option<Vec<u8>>)>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let absent = Entry {
                path: path.to_path_buf(),
                prior: Prior::Absent,
                prior_hash: None,
                mode: None,
                kind: Kind::File,
                link_target: None,
                after: None,
            };
            return Ok(Some((absent, None)));
        }
        Err(e) => return Err(e),
    };
    if meta.is_dir() {
        return Ok(None);
    }
    if meta.file_type().is_symlink() {
        let link = Entry {
            path: path.to_path_buf(),
            prior: Prior::File,
            prior_hash: None,
            mode: None,
            kind: Kind::Symlink,
            link_target: Some(std::fs::read_link(path)?),
            after: None,
        };
        return Ok(Some((link, None)));
    }
    if !meta.is_file() {
        tracing::warn!(path = %path.display(), "Recording a special file; rollback cannot recreate it");
        let special = Entry {
            path: path.to_path_buf(),
            prior: Prior::File,
            prior_hash: None,
            mode: None,
            kind: Kind::Special,
            link_target: None,
            after: None,
        };
        return Ok(Some((special, None)));
    }
    let bytes = std::fs::read(path)?;
    let entry = Entry {
        path: path.to_path_buf(),
        prior: Prior::File,
        prior_hash: Some(hash_content(&bytes)),
        mode: file_mode(&meta),
        kind: Kind::File,
        link_target: None,
        after: None,
    };
    Ok(Some((entry, Some(bytes))))
}

#[cfg(unix)]
fn file_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn file_mode(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{read_manifest, After, Kind};

    fn session_in(root: &Path) -> (WriteSession, PathBuf) {
        let dir = root.join("snap");
        (WriteSession::install(&dir).unwrap(), dir)
    }

    #[test]
    fn recording_outside_a_session_does_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, b"old").unwrap();
        record_before_write(&file).unwrap();
        record_tree_before_remove(tmp.path()).unwrap();
        assert!(!is_active());
    }

    #[test]
    fn existing_file_is_recorded_with_its_bytes_hash_and_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, b"old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        let (session, dir) = session_in(tmp.path());
        record_before_write(&file).unwrap();
        drop(session);

        let entries = read_manifest(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].prior, Prior::File);
        assert_eq!(
            entries[0].prior_hash.as_deref(),
            Some(hash_content(b"old").as_str())
        );
        #[cfg(unix)]
        assert_eq!(entries[0].mode, Some(0o640));
        let blob = dir.join(BLOB_DIR).join(hash_content(b"old"));
        assert_eq!(std::fs::read(blob).unwrap(), b"old");
    }

    #[test]
    fn missing_file_is_recorded_as_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("new.txt");
        let (session, dir) = session_in(tmp.path());
        record_before_write(&file).unwrap();
        drop(session);

        let entries = read_manifest(&dir).unwrap();
        assert_eq!(entries[0].prior, Prior::Absent);
        assert_eq!(entries[0].prior_hash, None);
    }

    #[test]
    fn only_the_first_touch_of_a_path_is_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, b"original").unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&file).unwrap();
        std::fs::write(&file, b"changed by sync").unwrap();
        record_before_write(&file).unwrap();
        drop(session);

        let entries = read_manifest(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].prior_hash.as_deref(),
            Some(hash_content(b"original").as_str())
        );
    }

    #[test]
    fn identical_content_in_two_files_is_stored_once() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        std::fs::write(&a, b"same").unwrap();
        std::fs::write(&b, b"same").unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&a).unwrap();
        record_before_write(&b).unwrap();
        drop(session);

        assert_eq!(read_manifest(&dir).unwrap().len(), 2);
        assert_eq!(std::fs::read_dir(dir.join(BLOB_DIR)).unwrap().count(), 1);
    }

    #[test]
    fn manifest_line_is_on_disk_before_the_session_ends() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, b"old").unwrap();
        let (_session, dir) = session_in(tmp.path());
        record_before_write(&file).unwrap();

        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();
        assert_eq!(text.lines().count(), 1);
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn a_directory_path_records_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(tmp.path()).unwrap();
        drop(session);
        assert!(
            !dir.exists(),
            "a session that recorded nothing leaves no directory"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_recorded_as_a_link_with_its_target_and_no_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        std::fs::write(tmp.path().join("target"), b"t").unwrap();
        std::os::unix::fs::symlink("target", &link).unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&link).unwrap();
        drop(session);

        let entries = read_manifest(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, Kind::Symlink);
        assert_eq!(entries[0].prior, Prior::File);
        assert_eq!(entries[0].link_target.as_deref(), Some(Path::new("target")));
        assert_eq!(entries[0].prior_hash, None);
        assert!(
            !dir.join(BLOB_DIR).exists(),
            "a link's target is not copied"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_dangling_symlink_is_recorded_as_a_link() {
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink("nowhere", &link).unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&link).unwrap();
        drop(session);
        assert_eq!(read_manifest(&dir).unwrap()[0].kind, Kind::Symlink);
    }

    #[cfg(unix)]
    #[test]
    fn a_special_file_is_recorded_as_special_without_reading_it() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("s.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&socket).unwrap();
        drop(session);

        let entries = read_manifest(&dir).unwrap();
        assert_eq!(entries[0].kind, Kind::Special);
        assert_eq!(entries[0].prior_hash, None);
        assert_eq!(entries[0].link_target, None);
    }

    #[test]
    fn a_relative_path_is_recorded_as_an_absolute_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(Path::new("surely-absent-relative-file.txt")).unwrap();
        drop(session);
        let entries = read_manifest(&dir).unwrap();
        assert!(entries[0].path.is_absolute());
    }

    #[test]
    fn a_path_with_parent_components_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (_session, _dir) = session_in(tmp.path());
        let escaping = tmp.path().join("a").join("..").join("b");
        let err = record_before_write(&escaping).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn the_sessions_own_files_are_never_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&dir.join(MANIFEST_FILE)).unwrap();
        drop(session);
        assert!(!dir.exists());
    }

    #[test]
    fn tree_removal_records_every_file_but_not_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let tree = tmp.path().join("tree");
        std::fs::create_dir_all(tree.join("sub")).unwrap();
        std::fs::write(tree.join("one"), b"1").unwrap();
        std::fs::write(tree.join("sub/two"), b"2").unwrap();
        let (session, dir) = session_in(tmp.path());
        record_tree_before_remove(&tree).unwrap();
        drop(session);
        assert_eq!(read_manifest(&dir).unwrap().len(), 2);
    }

    #[test]
    fn tree_removal_of_a_missing_path_records_nothing_and_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        let (session, dir) = session_in(tmp.path());
        record_tree_before_remove(&tmp.path().join("gone")).unwrap();
        drop(session);
        assert!(!dir.exists());
    }

    #[test]
    fn creating_directories_records_each_new_one_shallowest_first_as_an_absent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let deep = tmp.path().join("a").join("b").join("c");
        let (_session, dir) = session_in(tmp.path());
        record_dirs_before_create(&deep).unwrap();

        let entries = read_manifest(&dir).unwrap();
        let paths: Vec<_> = entries.iter().map(|e| e.path.clone()).collect();
        let a = tmp.path().join("a");
        assert_eq!(paths, [a.clone(), a.join("b"), a.join("b").join("c")]);
        assert!(entries
            .iter()
            .all(|e| e.prior == Prior::Absent && e.kind == Kind::Dir));
    }

    #[test]
    fn a_session_that_only_created_directories_leaves_no_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let (session, dir) = session_in(tmp.path());
        record_dirs_before_create(&tmp.path().join("empty")).unwrap();
        drop(session);
        assert!(!dir.exists());
    }

    #[test]
    fn creating_a_directory_that_exists_records_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let existing = tmp.path().join("there");
        std::fs::create_dir(&existing).unwrap();
        let (session, dir) = session_in(tmp.path());
        record_dirs_before_create(&existing).unwrap();
        drop(session);
        assert!(!dir.exists(), "nothing was recorded");
    }

    #[test]
    fn directories_inside_the_snapshot_are_not_recorded() {
        let tmp = tempfile::tempdir().unwrap();
        let (session, dir) = session_in(tmp.path());
        record_dirs_before_create(&dir.join(BLOB_DIR).join("sub")).unwrap();
        drop(session);
        assert!(!dir.exists(), "nothing was recorded");
    }

    #[test]
    fn creating_directories_outside_a_session_records_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        record_dirs_before_create(&tmp.path().join("x/y")).unwrap();
        assert!(!tmp.path().join("x").exists(), "recording never creates");
    }

    #[test]
    fn a_directory_path_with_parent_components_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let (_session, _dir) = session_in(tmp.path());
        let escaping = tmp.path().join("a").join("..").join("b");
        let err = record_dirs_before_create(&escaping).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_completed_write_appends_the_after_hash_of_its_final_content() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, b"old").unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&file).unwrap();
        record_after_write(&file, b"first").unwrap();
        record_after_write(&file, b"final").unwrap();
        drop(session);

        let entries = read_manifest(&dir).unwrap();
        assert_eq!(entries.len(), 1, "after lines are not entries of their own");
        assert_eq!(entries[0].after, Some(After::File(hash_content(b"final"))));
    }

    #[test]
    fn a_removal_after_a_write_leaves_the_path_absent_after_the_sync() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("a.txt");
        let (session, dir) = session_in(tmp.path());
        record_before_write(&file).unwrap();
        record_after_write(&file, b"made").unwrap();
        record_after_remove(&file).unwrap();
        drop(session);
        assert_eq!(read_manifest(&dir).unwrap()[0].after, Some(After::Absent));
    }

    #[test]
    fn creating_a_directory_where_the_sync_removed_a_file_marks_it_a_directory_after() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("asset");
        std::fs::write(&path, b"file").unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&path).unwrap();
        record_after_remove(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        record_dirs_before_create(&path).unwrap();
        drop(session);

        let entries = read_manifest(&dir).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, Kind::File);
        assert_eq!(entries[0].after, Some(After::Dir));
    }

    #[test]
    fn an_after_hash_for_a_path_never_recorded_is_not_written() {
        let tmp = tempfile::tempdir().unwrap();
        let (_session, dir) = session_in(tmp.path());
        record_after_write(&tmp.path().join("unseen"), b"x").unwrap();
        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn after_hashes_outside_a_session_do_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        record_after_write(&tmp.path().join("f"), b"x").unwrap();
        record_after_remove(&tmp.path().join("f")).unwrap();
        record_after_copy(&tmp.path().join("f")).unwrap();
        assert!(!is_active());
    }

    #[test]
    fn a_copy_appends_the_hash_of_the_file_it_produced() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("copied");
        let (session, dir) = session_in(tmp.path());
        record_before_write(&file).unwrap();
        std::fs::write(&file, b"copied bytes").unwrap();
        record_after_copy(&file).unwrap();
        drop(session);
        assert_eq!(
            read_manifest(&dir).unwrap()[0].after,
            Some(After::File(hash_content(b"copied bytes")))
        );
    }

    #[test]
    #[should_panic(expected = "nesting must be explicit")]
    fn installing_a_second_session_panics() {
        let tmp = tempfile::tempdir().unwrap();
        let (_outer, _dir) = session_in(tmp.path());
        let _ = WriteSession::install(&tmp.path().join("second"));
    }

    #[test]
    fn dropping_the_session_deactivates_recording_and_allows_a_new_one() {
        let tmp = tempfile::tempdir().unwrap();
        let (first, _dir) = session_in(tmp.path());
        assert!(is_active());
        drop(first);
        assert!(!is_active());
        let second = WriteSession::install(&tmp.path().join("again")).unwrap();
        assert!(is_active());
        drop(second);
    }

    #[test]
    fn install_fails_when_the_directory_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let err = WriteSession::install(tmp.path()).err().unwrap();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        assert!(!is_active());
    }

    #[cfg(unix)]
    #[test]
    fn session_directory_blobs_and_manifest_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("secret.json");
        std::fs::write(&file, b"{\"token\":1}").unwrap();
        let (session, dir) = session_in(tmp.path());
        record_before_write(&file).unwrap();
        drop(session);

        let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.clone()), 0o700);
        assert_eq!(mode(dir.join(BLOB_DIR)), 0o700);
        assert_eq!(mode(dir.join(MANIFEST_FILE)), 0o600);
        assert_eq!(
            mode(dir.join(BLOB_DIR).join(hash_content(b"{\"token\":1}"))),
            0o600
        );
    }
}
