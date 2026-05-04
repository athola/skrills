//! Integration tests for the operational kill-switch (B4 from PR #218 review).
//!
//! When the cold-window kill-switch is engaged, every adapter's mutating
//! `write_*` method must refuse with [`SyncError::TokenBudgetExceeded`].
//! These tests fail today (no caller exists) and pass once each adapter
//! consults the switch at the top of every write entry-point.
//!
//! NI15 (Cursor adapter swallowing per-entry I/O errors) is exercised by
//! [`cursor_pruning_surfaces_warning_on_unreadable_subentry`], which seeds
//! a Unix-mode-0 directory entry so the read of `plugins/local` itself
//! succeeds but iteration over the entries surfaces an error that must
//! land in the report's warning list (no longer silently dropped).

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::SystemTime;

use skrills_sync::{
    AgentAdapter, ClaudeAdapter, CodexAdapter, Command, CopilotAdapter, CursorAdapter, KillSwitch,
    McpServer, Preferences, SyncError,
};
use tempfile::TempDir;

fn engaged() -> KillSwitch {
    let s = KillSwitch::new();
    s.engage();
    s
}

fn dummy_command(name: &str) -> Command {
    Command {
        name: name.to_string(),
        content: format!("# {name}\n").into_bytes(),
        source_path: PathBuf::from(format!("/tmp/{name}.md")),
        modified: SystemTime::now(),
        hash: "dummy".to_string(),
        modules: Vec::new(),
        content_format: Default::default(),
        plugin_origin: None,
    }
}

fn assert_token_budget_exceeded(err: anyhow::Error) {
    let downcast = err
        .downcast_ref::<SyncError>()
        .unwrap_or_else(|| panic!("expected SyncError, got: {err:?}"));
    assert!(
        matches!(downcast, SyncError::TokenBudgetExceeded { .. }),
        "expected TokenBudgetExceeded, got: {downcast:?}"
    );
}

// ---------- Claude ----------

#[test]
fn claude_write_commands_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_commands(&[dummy_command("a")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn claude_write_skills_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_skills(&[dummy_command("s")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn claude_write_mcp_servers_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter
        .write_mcp_servers(&HashMap::<String, McpServer>::new())
        .unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn claude_write_preferences_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = ClaudeAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter
        .write_preferences(&Preferences::default())
        .unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn claude_disengaged_switch_does_not_refuse() {
    // Sanity: the guard fires only on ENGAGED switches.
    let tmp = TempDir::new().unwrap();
    let adapter =
        ClaudeAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(KillSwitch::new());
    // Empty input should succeed.
    adapter.write_commands(&[]).expect("disengaged ok");
}

// ---------- Codex ----------

#[test]
fn codex_write_commands_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CodexAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_commands(&[dummy_command("a")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn codex_write_skills_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CodexAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_skills(&[dummy_command("s")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn codex_write_agents_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CodexAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_agents(&[dummy_command("a")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn codex_write_mcp_servers_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CodexAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter
        .write_mcp_servers(&HashMap::<String, McpServer>::new())
        .unwrap_err();
    assert_token_budget_exceeded(err);
}

// ---------- Copilot ----------

#[test]
fn copilot_write_skills_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_skills(&[dummy_command("s")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn copilot_write_mcp_servers_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter
        .write_mcp_servers(&HashMap::<String, McpServer>::new())
        .unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn copilot_write_preferences_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter
        .write_preferences(&Preferences::default())
        .unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn copilot_write_agents_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_agents(&[dummy_command("a")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

// ---------- Cursor ----------

#[test]
fn cursor_write_skills_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_skills(&[dummy_command("s")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn cursor_write_commands_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter.write_commands(&[dummy_command("a")]).unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn cursor_write_mcp_servers_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter
        .write_mcp_servers(&HashMap::<String, McpServer>::new())
        .unwrap_err();
    assert_token_budget_exceeded(err);
}

#[test]
fn cursor_write_instructions_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let err = adapter
        .write_instructions(&[dummy_command("rule")])
        .unwrap_err();
    assert_token_budget_exceeded(err);
}

// ---------- NI15: cursor pruning surfaces I/O errors ----------

/// Seeds `~/.cursor/plugins/local/<name>` with a directory whose mode is
/// `0o000`, so `fs::read_dir` succeeds for the parent but iteration over
/// the entries returns an `Err`. The adapter must surface that error as a
/// warning on the report, not silently drop it.
///
/// Gated to Unix because Windows file-permission semantics are different
/// and the equivalent surface (a path traversal failure) is awkward to
/// produce from a test.
#[cfg(unix)]
#[test]
fn cursor_pruning_surfaces_warning_on_unreadable_subentry() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    let tmp = TempDir::new().unwrap();
    let root = tmp.path();

    // Build the cursor `plugins/local` directory and a nested directory
    // named "stale-plugin". We don't reference it in the asset list, so
    // pruning will see it as a candidate to remove.
    let local = root.join("plugins").join("local");
    let stale = local.join("stale-plugin");
    fs::create_dir_all(&stale).unwrap();

    // Make `stale-plugin` mode 0 so `remove_dir_all` fails with EACCES /
    // similar. The directory entry itself is still listable from the
    // parent, but the recursive remove inside surfaces an I/O error that
    // must land in `report.warnings`.
    let mut perms = fs::metadata(&stale).unwrap().permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&stale, perms).unwrap();

    // Defer perm restoration so the tempdir cleanup doesn't fail when the
    // test finishes — this is best-effort and runs even on test panic.
    struct PermsGuard<'a>(&'a Path);
    impl Drop for PermsGuard<'_> {
        fn drop(&mut self) {
            if let Ok(meta) = fs::metadata(self.0) {
                let mut p = meta.permissions();
                p.set_mode(0o755);
                let _ = fs::set_permissions(self.0, p);
            }
        }
    }
    let _guard = PermsGuard(&stale);

    // Run with NO matching plugin asset, so the pruning loop tries to
    // remove `stale-plugin` and fails. (Empty asset list still triggers
    // the read_dir + prune branch because `assets.is_empty()` returns
    // early on `Ok(report)`. To force the prune branch, supply a
    // non-empty asset list referencing a different plugin.)
    let adapter = CursorAdapter::with_root(root.to_path_buf());

    use skrills_sync::PluginAsset;
    let asset = PluginAsset::new(
        "fresh-plugin".to_string(),
        "marketplace".to_string(),
        "1.0.0".to_string(),
        PathBuf::from(".claude-plugin/plugin.json"),
        b"{}".to_vec(),
        false,
    );

    let report = adapter
        .write_plugin_assets(&[asset])
        .expect("write_plugin_assets returns Ok with warnings on partial failure");

    assert!(
        report.warnings.iter().any(|w| w.contains("stale-plugin")),
        "expected warning surfacing the stale-plugin failure, got: {:?}",
        report.warnings
    );
}
