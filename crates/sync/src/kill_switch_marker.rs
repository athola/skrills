//! The cold-window kill switch as other processes see it (FR12).
//!
//! [`KillSwitch`](skrills_snapshot::KillSwitch) lives in memory, and
//! `skrills cold-window` runs in its own process, so a `sync-all` or an MCP
//! sync never saw it engage. While its switch is engaged, cold-window holds
//! an [`EngagedMarker`]: a file at [`marker_path`] holding its pid. A sync
//! refuses while that file names a live process. A marker whose process has
//! died (a crash, `kill -9`) is stale; it is removed and ignored.
//!
//! A pid the system has reused for another process makes a stale marker look
//! live until the file is removed by hand.

use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::error::SyncError;
use crate::Result;

/// File name of the marker under `~/.skrills`.
const MARKER_FILE: &str = "kill-switch.pid";

/// Absolute path that replaces [`marker_path`]'s default. The Makefile test
/// targets set it, so a developer's running cold-window does not make the
/// test suite's syncs refuse.
pub const MARKER_ENV: &str = "SKRILLS_KILL_SWITCH_FILE";

/// `~/.skrills/kill-switch.pid` (or an absolute [`MARKER_ENV`]), or `None`
/// when no home directory is known.
///
/// Home is resolved as `skrills_state::home_dir` does it: an absolute `HOME`
/// first, then the platform lookup. That crate is not a dependency here
/// because it brings SQLite with it.
pub fn marker_path() -> Option<PathBuf> {
    if let Some(file) = std::env::var_os(MARKER_ENV).map(PathBuf::from) {
        if file.is_absolute() {
            return Some(file);
        }
    }
    #[cfg(unix)]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if home.is_absolute() {
            return Some(home.join(".skrills").join(MARKER_FILE));
        }
    }
    dirs::home_dir()
        .filter(|h| h.is_absolute())
        .map(|h| h.join(".skrills").join(MARKER_FILE))
}

/// The marker of an engaged kill switch. Dropping it removes the file, so a
/// normal exit, an error return and an aborted task all clear it.
#[derive(Debug)]
pub struct EngagedMarker {
    path: PathBuf,
}

impl EngagedMarker {
    /// Writes this process's pid to `path`, creating its directory.
    pub fn create(path: &Path) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, std::process::id().to_string())?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for EngagedMarker {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    path = %self.path.display(),
                    error = %e,
                    "Could not remove the kill-switch marker; syncs refuse until it is deleted"
                );
            }
        }
    }
}

/// The pid of the live process holding the marker at `path`, if any.
///
/// A marker that names a dead process, or holds no pid, is removed.
pub fn live_holder(path: &Path) -> Option<u32> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "Could not read the kill-switch marker; treating the switch as released"
            );
            return None;
        }
    };
    if let Some(pid) = text
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|&p| process_alive(p))
    {
        return Some(pid);
    }
    tracing::info!(path = %path.display(), "Removing a stale kill-switch marker");
    if let Err(e) = std::fs::remove_file(path) {
        tracing::warn!(path = %path.display(), error = %e, "Could not remove a stale kill-switch marker");
    }
    None
}

/// Refuses with [`SyncError::KillSwitchEngaged`] while a live process holds
/// the marker at `path`. No path (no home directory) means no marker.
pub(crate) fn ensure_not_engaged(path: Option<&Path>) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    match live_holder(path) {
        None => Ok(()),
        Some(pid) => Err(SyncError::KillSwitchEngaged).with_context(|| {
            format!(
                "cold-window (pid {pid}) engaged the kill switch; it is released when that \
                 process exits ({})",
                path.display()
            )
        }),
    }
}

/// Whether a process with this pid exists, asked without `unsafe` through
/// the system's own tool.
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    pid != 0
        && std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

#[cfg(not(unix))]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_lives_under_the_skrills_dir_of_home() {
        let _lock = skrills_test_utils::env_guard();
        let fixture = skrills_test_utils::TestFixture::new().unwrap();
        let _home = fixture.home_guard();
        let _no_override = skrills_test_utils::set_env_var(MARKER_ENV, None);

        let path = marker_path().unwrap();

        assert!(path.ends_with(".skrills/kill-switch.pid"), "{path:?}");
        #[cfg(unix)]
        assert!(path.starts_with(fixture.home_path()), "{path:?}");
    }

    #[test]
    fn marker_path_follows_the_env_override() {
        let _lock = skrills_test_utils::env_guard();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("switch.pid");
        let _override = skrills_test_utils::set_env_var(MARKER_ENV, file.to_str());

        assert_eq!(marker_path(), Some(file));
    }

    #[test]
    fn a_relative_override_is_ignored() {
        let _lock = skrills_test_utils::env_guard();
        let fixture = skrills_test_utils::TestFixture::new().unwrap();
        let _home = fixture.home_guard();
        let _override = skrills_test_utils::set_env_var(MARKER_ENV, Some("rel/switch.pid"));

        let path = marker_path().unwrap();

        assert!(path.ends_with(".skrills/kill-switch.pid"), "{path:?}");
    }
}
