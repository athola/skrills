//! The cold-window kill switch reaches syncs run by other processes through
//! a marker file holding the engaging process's pid (FR12, M451).

use std::fs;
use std::path::{Path, PathBuf};

use skrills_sync::kill_switch_marker::{live_holder, EngagedMarker};
use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncError, SyncOrchestrator, SyncParams};
use tempfile::TempDir;

/// A pid no process can have on any supported system.
const DEAD_PID: &str = "2147483647";

fn seed_command(claude_root: &Path) {
    let dir = claude_root.join("commands");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("hello.md"), "# Hello").unwrap();
}

fn commands_only() -> SyncParams {
    SyncParams {
        sync_commands: true,
        sync_skills: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_hooks: false,
        sync_instructions: false,
        sync_plugin_assets: false,
        ..Default::default()
    }
}

/// Runs a Claude -> Codex command sync that consults `marker`.
fn sync_with_marker(tmp: &Path, marker: PathBuf) -> anyhow::Result<skrills_sync::SyncReport> {
    seed_command(&tmp.join("claude"));
    let source = ClaudeAdapter::with_root(tmp.join("claude"));
    let target = CodexAdapter::with_root(tmp.join("codex"));
    SyncOrchestrator::new(source, target)
        .with_kill_switch_marker(Some(marker))
        .sync(&commands_only())
}

#[test]
fn sync_is_refused_while_a_live_process_holds_the_marker() {
    let tmp = TempDir::new().unwrap();
    let marker = tmp.path().join("kill-switch.pid");
    fs::write(&marker, std::process::id().to_string()).unwrap();

    let err = sync_with_marker(tmp.path(), marker.clone()).unwrap_err();

    assert_eq!(
        err.downcast_ref::<SyncError>(),
        Some(&SyncError::KillSwitchEngaged),
        "{err:#}"
    );
    assert!(!tmp.path().join("codex/prompts/hello.md").exists());
    assert!(marker.exists(), "a live marker is left alone");
}

#[test]
fn sync_proceeds_without_a_marker() {
    let tmp = TempDir::new().unwrap();

    let report = sync_with_marker(tmp.path(), tmp.path().join("kill-switch.pid")).unwrap();

    assert!(report.success);
    assert!(tmp.path().join("codex/prompts/hello.md").exists());
}

#[test]
fn a_marker_left_by_a_dead_process_is_ignored_and_removed() {
    let tmp = TempDir::new().unwrap();
    let marker = tmp.path().join("kill-switch.pid");
    fs::write(&marker, DEAD_PID).unwrap();

    let report = sync_with_marker(tmp.path(), marker.clone()).unwrap();

    assert!(report.success);
    assert!(tmp.path().join("codex/prompts/hello.md").exists());
    assert!(!marker.exists());
}

#[test]
fn a_marker_without_a_pid_is_stale() {
    let tmp = TempDir::new().unwrap();
    let marker = tmp.path().join("kill-switch.pid");
    fs::write(&marker, "not a pid").unwrap();

    assert_eq!(live_holder(&marker), None);
    assert!(!marker.exists());
}

/// A dry run writes nothing, so the switch does not stop it.
#[test]
fn a_dry_run_still_previews_while_the_switch_is_engaged() {
    let tmp = TempDir::new().unwrap();
    let marker = tmp.path().join("kill-switch.pid");
    fs::write(&marker, std::process::id().to_string()).unwrap();
    seed_command(&tmp.path().join("claude"));

    let report = SyncOrchestrator::new(
        ClaudeAdapter::with_root(tmp.path().join("claude")),
        CodexAdapter::with_root(tmp.path().join("codex")),
    )
    .with_kill_switch_marker(Some(marker))
    .sync(&SyncParams {
        dry_run: true,
        ..commands_only()
    })
    .unwrap();

    assert_eq!(report.commands.written, 1);
    assert!(!tmp.path().join("codex/prompts/hello.md").exists());
}

#[test]
fn engaging_writes_the_pid_and_releasing_removes_the_marker() {
    let tmp = TempDir::new().unwrap();
    let marker = tmp.path().join("nested/kill-switch.pid");

    let held = EngagedMarker::create(&marker).unwrap();
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        std::process::id().to_string()
    );
    assert_eq!(live_holder(&marker), Some(std::process::id()));

    drop(held);
    assert!(!marker.exists());
}
