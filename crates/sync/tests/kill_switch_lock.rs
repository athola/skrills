//! Integration tests for the operational kill-switch.
//!
//! When the cold-window kill-switch is engaged, every adapter's mutating
//! `write_*` method must refuse with [`SyncError::TokenBudgetExceeded`].
//! These tests fail today (no caller exists) and pass once each adapter
//! consults the switch at the top of every write entry-point.
//!
//! Cursor adapter per-entry I/O errors are exercised by
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

// ---------- Cursor pruning surfaces I/O errors ----------

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
    // Only a directory carrying the mirror marker is a prune candidate.
    fs::write(stale.join(".skrills-mirror"), b"marker").unwrap();
    fs::write(stale.join("inner.txt"), b"content").unwrap();

    // Drop write permission on `stale-plugin` so `remove_dir_all` fails with
    // EACCES when it tries to unlink `inner.txt`. The marker stays statable, so
    // the directory is still recognized as a prune candidate and the I/O error
    // has to land in `report.warnings`.
    let mut perms = fs::metadata(&stale).unwrap().permissions();
    perms.set_mode(0o555);
    fs::set_permissions(&stale, perms).unwrap();

    // Defer perm restoration so the tempdir cleanup doesn't fail when the
    // test finishes, this is best-effort and runs even on test panic.
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
    // the read_dir and prune branch because `assets.is_empty()` returns
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

// ---------- Every adapter x every write ----------

type Write = fn(&dyn AgentAdapter) -> anyhow::Result<skrills_sync::WriteReport>;

/// Every mutating entry point, with input that would really write.
fn every_write() -> Vec<(&'static str, Write)> {
    vec![
        ("write_commands", |a| {
            a.write_commands(&[dummy_command("c")])
        }),
        ("write_skills", |a| a.write_skills(&[dummy_command("s")])),
        ("write_agents", |a| a.write_agents(&[dummy_command("a")])),
        ("write_hooks", |a| {
            let mut hook = dummy_command("PreToolUse");
            hook.content = br#"[{"command": "x"}]"#.to_vec();
            a.write_hooks(&[hook])
        }),
        ("write_instructions", |a| {
            a.write_instructions(&[dummy_command("rule")])
        }),
        ("write_mcp_servers", |a| {
            a.write_mcp_servers(&HashMap::<String, McpServer>::new())
        }),
        ("write_preferences", |a| {
            a.write_preferences(&Preferences::default())
        }),
    ]
}

fn engaged_adapters(root: &std::path::Path) -> Vec<Box<dyn AgentAdapter>> {
    let root = root.to_path_buf();
    vec![
        Box::new(ClaudeAdapter::with_root(root.join("claude")).with_kill_switch(engaged())),
        Box::new(CodexAdapter::with_root(root.join("codex")).with_kill_switch(engaged())),
        Box::new(CopilotAdapter::with_root(root.join("copilot")).with_kill_switch(engaged())),
        Box::new(CursorAdapter::with_root(root.join("cursor")).with_kill_switch(engaged())),
    ]
}

#[test]
fn every_adapter_write_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let mut missing = Vec::new();
    for adapter in engaged_adapters(tmp.path()) {
        for (method, write) in every_write() {
            match write(adapter.as_ref()) {
                Err(err)
                    if err
                        .downcast_ref::<SyncError>()
                        .is_some_and(|e| matches!(e, SyncError::TokenBudgetExceeded { .. })) => {}
                other => missing.push(format!("{}::{method} -> {other:?}", adapter.name())),
            }
        }
    }
    assert!(
        missing.is_empty(),
        "writes that ignored the switch: {missing:#?}"
    );
}

#[test]
fn cursor_write_plugin_assets_refuses_when_engaged() {
    let tmp = TempDir::new().unwrap();
    let adapter = CursorAdapter::with_root(tmp.path().to_path_buf()).with_kill_switch(engaged());
    let asset = skrills_sync::PluginAsset::new(
        "p".to_string(),
        "m".to_string(),
        "1.0.0".to_string(),
        PathBuf::from(".claude-plugin/plugin.json"),
        b"{}".to_vec(),
        false,
    );
    let err = adapter.write_plugin_assets(&[asset]).unwrap_err();
    assert_token_budget_exceeded(err);
}

/// The orchestrator ends the run on the first refusal; no later phase may
/// write anything into the target.
#[test]
fn orchestrator_sync_stops_and_writes_nothing_when_the_switch_is_engaged() {
    use skrills_sync::{SyncOrchestrator, SyncParams};

    let source = TempDir::new().unwrap();
    std::fs::create_dir_all(source.path().join("commands")).unwrap();
    std::fs::write(source.path().join("commands/c.md"), "# c\n").unwrap();
    let skill = source.path().join("skills/s");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: s\ndescription: d\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(source.path().join("settings.json"), r#"{"model": "opus"}"#).unwrap();
    std::fs::write(
        source.path().join(".claude.json"),
        r#"{"mcpServers": {"m": {"command": "x"}}}"#,
    )
    .unwrap();
    let target = TempDir::new().unwrap();

    let orchestrator = SyncOrchestrator::new(
        ClaudeAdapter::with_root(source.path().to_path_buf()),
        CursorAdapter::with_root(target.path().to_path_buf()).with_kill_switch(engaged()),
    );
    let err = orchestrator.sync(&SyncParams::default()).unwrap_err();

    assert_token_budget_exceeded(err);
    let written: Vec<_> = walkdir::WalkDir::new(target.path())
        .min_depth(1)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.path().display().to_string())
        .collect();
    assert!(
        written.is_empty(),
        "files written past the switch: {written:?}"
    );
}
