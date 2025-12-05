//! Integration tests for cross-agent sync functionality.

use skrills_sync::{ClaudeAdapter, CodexAdapter, SyncOrchestrator, SyncParams};
use std::fs;
use tempfile::tempdir;

/// Creates a mock Claude configuration directory with sample data.
fn setup_claude_config(root: &std::path::Path) {
    // Create commands directory with a sample command
    let cmd_dir = root.join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("test-cmd.md"), "# Test Command\nDo something").unwrap();

    // Create settings.json with MCP servers and preferences
    let settings = serde_json::json!({
        "mcpServers": {
            "test-server": {
                "command": "/usr/bin/test-server",
                "args": ["--port", "8080"]
            }
        },
        "model": "claude-3-5-sonnet"
    });
    fs::write(
        root.join("settings.json"),
        serde_json::to_string_pretty(&settings).unwrap(),
    )
    .unwrap();
}

#[test]
fn full_sync_claude_to_codex() {
    let claude_dir = tempdir().unwrap();
    let codex_dir = tempdir().unwrap();

    setup_claude_config(claude_dir.path());

    let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
    let target = CodexAdapter::with_root(codex_dir.path().to_path_buf());

    let params = SyncParams {
        from: Some("claude".to_string()),
        dry_run: false,
        sync_commands: true,
        sync_mcp_servers: true,
        sync_preferences: true,
        sync_skills: false,
        ..Default::default()
    };

    let orch = SyncOrchestrator::new(source, target);
    let report = orch.sync(&params).unwrap();

    // Verify sync results
    assert!(report.success);
    assert_eq!(report.commands.written, 1);
    assert_eq!(report.mcp_servers.written, 1);
    assert_eq!(report.preferences.written, 1);

    // Verify files were created
    let cmd_file = codex_dir.path().join("commands/test-cmd.md");
    assert!(cmd_file.exists());
    assert_eq!(
        fs::read_to_string(&cmd_file).unwrap(),
        "# Test Command\nDo something"
    );

    // Verify config.json was created with MCP servers
    let config_file = codex_dir.path().join("config.json");
    assert!(config_file.exists());
    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&config_file).unwrap()).unwrap();
    assert!(config["mcpServers"]["test-server"].is_object());
    assert_eq!(config["model"], "claude-3-5-sonnet");
}

#[test]
fn dry_run_makes_no_changes() {
    let claude_dir = tempdir().unwrap();
    let codex_dir = tempdir().unwrap();

    setup_claude_config(claude_dir.path());

    let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
    let target = CodexAdapter::with_root(codex_dir.path().to_path_buf());

    let params = SyncParams {
        from: Some("claude".to_string()),
        dry_run: true,
        sync_commands: true,
        sync_mcp_servers: true,
        sync_preferences: true,
        sync_skills: false,
        ..Default::default()
    };

    let orch = SyncOrchestrator::new(source, target);
    let report = orch.sync(&params).unwrap();

    // Report should indicate what would be synced
    assert_eq!(report.commands.written, 1);
    assert_eq!(report.mcp_servers.written, 1);

    // But no files should be created
    let cmd_file = codex_dir.path().join("commands/test-cmd.md");
    assert!(!cmd_file.exists());
    let config_file = codex_dir.path().join("config.json");
    assert!(!config_file.exists());
}

#[test]
fn bidirectional_sync_codex_to_claude() {
    let claude_dir = tempdir().unwrap();
    let codex_dir = tempdir().unwrap();

    // Set up Codex config as source
    let cmd_dir = codex_dir.path().join("commands");
    fs::create_dir_all(&cmd_dir).unwrap();
    fs::write(cmd_dir.join("codex-cmd.md"), "# Codex Command").unwrap();

    let config = serde_json::json!({
        "mcpServers": {
            "codex-server": {
                "command": "/bin/codex-server"
            }
        },
        "model": "gpt-4o"
    });
    fs::write(
        codex_dir.path().join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();

    let source = CodexAdapter::with_root(codex_dir.path().to_path_buf());
    let target = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());

    let params = SyncParams {
        from: Some("codex".to_string()),
        dry_run: false,
        sync_commands: true,
        sync_mcp_servers: true,
        sync_preferences: true,
        sync_skills: false,
        ..Default::default()
    };

    let orch = SyncOrchestrator::new(source, target);
    let report = orch.sync(&params).unwrap();

    assert!(report.success);
    assert_eq!(report.commands.written, 1);
    assert_eq!(report.mcp_servers.written, 1);

    // Verify Claude directory was populated
    let cmd_file = claude_dir.path().join("commands/codex-cmd.md");
    assert!(cmd_file.exists());

    let settings_file = claude_dir.path().join("settings.json");
    assert!(settings_file.exists());
}

#[test]
fn selective_sync_commands_only() {
    let claude_dir = tempdir().unwrap();
    let codex_dir = tempdir().unwrap();

    setup_claude_config(claude_dir.path());

    let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
    let target = CodexAdapter::with_root(codex_dir.path().to_path_buf());

    let params = SyncParams {
        from: Some("claude".to_string()),
        dry_run: false,
        sync_commands: true,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_skills: false,
        ..Default::default()
    };

    let orch = SyncOrchestrator::new(source, target);
    let report = orch.sync(&params).unwrap();

    // Only commands should be synced
    assert_eq!(report.commands.written, 1);
    assert_eq!(report.mcp_servers.written, 0);
    assert_eq!(report.preferences.written, 0);

    // Command file should exist
    let cmd_file = codex_dir.path().join("commands/test-cmd.md");
    assert!(cmd_file.exists());

    // But config.json should NOT exist (no MCP servers or prefs synced)
    let config_file = codex_dir.path().join("config.json");
    assert!(!config_file.exists());
}

#[test]
fn sync_skips_unchanged_content() {
    let claude_dir = tempdir().unwrap();
    let codex_dir = tempdir().unwrap();

    setup_claude_config(claude_dir.path());

    let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
    let target = CodexAdapter::with_root(codex_dir.path().to_path_buf());

    let params = SyncParams {
        from: Some("claude".to_string()),
        dry_run: false,
        sync_commands: true,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_skills: false,
        ..Default::default()
    };

    let orch = SyncOrchestrator::new(source, target);

    // First sync
    let report1 = orch.sync(&params).unwrap();
    assert_eq!(report1.commands.written, 1);

    // Second sync should skip unchanged
    let report2 = orch.sync(&params).unwrap();
    assert_eq!(report2.commands.written, 0);
    assert_eq!(report2.commands.skipped.len(), 1);
}

#[test]
fn report_contains_summary() {
    let claude_dir = tempdir().unwrap();
    let codex_dir = tempdir().unwrap();

    setup_claude_config(claude_dir.path());

    let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
    let target = CodexAdapter::with_root(codex_dir.path().to_path_buf());

    let params = SyncParams::default();
    let orch = SyncOrchestrator::new(source, target);
    let report = orch.sync(&params).unwrap();

    assert!(report.summary.contains("claude"));
    assert!(report.summary.contains("codex"));
}
