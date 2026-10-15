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

/// `~/.skrills/kill-switch.pid`, or `None` when no home directory is known.
///
/// Home is resolved as `skrills_state::home_dir` does it: an absolute `HOME`
/// first, then the platform lookup. That crate is not a dependency here
/// because it brings SQLite with it.
pub fn marker_path() -> Option<PathBuf> {
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
    #[serial_test::serial]
    fn marker_lives_under_the_skrills_dir_of_home() {
        let home = tempfile::tempdir().unwrap();
        let saved = std::env::var_os("HOME");
        std::env::set_var("HOME", home.path());

        let path = marker_path();

        match saved {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        #[cfg(unix)]
        assert_eq!(path, Some(home.path().join(".skrills/kill-switch.pid")));
        #[cfg(not(unix))]
        assert!(path.is_some_and(|p| p.ends_with(".skrills/kill-switch.pid")));
    }
}
