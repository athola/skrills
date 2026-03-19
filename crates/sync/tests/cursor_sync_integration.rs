//! Integration tests for Cursor sync functionality.
//!
//! These tests validate bidirectional sync operations between Cursor and
//! Claude/Codex/Copilot, covering all artifact types:
//!
//! - Skills: Frontmatter stripping (Claude→Cursor), module preservation
//! - Commands: Near-1:1 copy
//! - Agents: Field translation (background↔is_background, model mapping)
//! - Hooks: Event name mapping (PascalCase↔camelCase), unmappable event skipping
//! - Rules: .mdc format generation, CLAUDE.md→always-apply, glob preservation
//! - MCP: .cursor/mcp.json read/write
//! - Round-trip fidelity: Claude→Cursor→Claude content preservation

use skrills_sync::{
    adapters::traits::AgentAdapter,
    adapters::{ClaudeAdapter, CursorAdapter},
    orchestrator::{SyncOrchestrator, SyncParams},
};
use std::fs;
use tempfile::TempDir;

/// Test fixture for Cursor sync operations.
struct CursorSyncTestSetup {
    cursor_dir: TempDir,
    claude_dir: TempDir,
}

impl CursorSyncTestSetup {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            cursor_dir: TempDir::new()?,
            claude_dir: TempDir::new()?,
        })
    }

    /// Create mock Claude configuration with all artifact types.
    fn setup_claude_config(&self) {
        let root = self.claude_dir.path();

        // Skills with frontmatter + modules
        let skills_dir = root.join("skills");
        fs::create_dir_all(skills_dir.join("code-review/modules")).unwrap();
        fs::write(
            skills_dir.join("code-review/SKILL.md"),
            "---\nname: code-review\ndescription: Reviews code thoroughly\ncategory: quality\ntags:\n  - review\n  - quality\n---\n\n# Code Review\n\nReview code for bugs, style, and performance.\n",
        ).unwrap();
        fs::write(
            skills_dir.join("code-review/modules/checklist.md"),
            "# Review Checklist\n\n- [ ] Check error handling\n- [ ] Check edge cases\n",
        )
        .unwrap();

        fs::create_dir_all(skills_dir.join("deploy")).unwrap();
        fs::write(
            skills_dir.join("deploy/SKILL.md"),
            "---\nname: deploy\ndescription: Deploy to staging\n---\n\n# Deploy\n\n1. Run tests\n2. Build\n3. Deploy\n",
        ).unwrap();

        // Commands
        let cmd_dir = root.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(
            cmd_dir.join("commit-msg.md"),
            "# Commit Message\n\nGenerate a conventional commit message.\n",
        )
        .unwrap();
        fs::write(
            cmd_dir.join("review-pr.md"),
            "# Review PR\n\nReview the current pull request.\n",
        )
        .unwrap();

        // Agents with Claude-specific fields
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("reviewer.md"),
            "---\nname: reviewer\ndescription: Code review specialist\nbackground: true\ntools: [Read, Write, Bash]\nisolation: worktree\nmodel: claude-sonnet-4-6\n---\n\nYou are a code reviewer. Review thoroughly.\n",
        ).unwrap();
        fs::write(
            agents_dir.join("builder.md"),
            "---\nname: builder\ndescription: Builds and tests code\nmodel: claude-opus-4-6\n---\n\nYou build and test code.\n",
        ).unwrap();

        // Instructions (CLAUDE.md)
        fs::write(
            root.join("CLAUDE.md"),
            "# Project Instructions\n\n- Use TypeScript\n- Follow TDD\n- Write clean code\n",
        )
        .unwrap();

        // MCP config — Claude reads from settings.json
        let settings = serde_json::json!({
            "mcpServers": {
                "skrills": {
                    "command": "/usr/bin/skrills",
                    "args": ["serve"],
                    "env": {"SKRILLS_LOG": "info"}
                }
            }
        });
        fs::write(
            root.join("settings.json"),
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();
    }

    /// Create mock Cursor configuration with all artifact types.
    fn setup_cursor_config(&self) {
        let root = self.cursor_dir.path();

        // Skills (no frontmatter)
        let skills_dir = root.join("skills");
        fs::create_dir_all(skills_dir.join("test-runner")).unwrap();
        fs::write(
            skills_dir.join("test-runner/SKILL.md"),
            "# Test Runner\n\nRun the test suite and report results.\n",
        )
        .unwrap();

        // Commands
        let cmd_dir = root.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(
            cmd_dir.join("lint.md"),
            "# Lint\n\nRun the linter on all files.\n",
        )
        .unwrap();

        // Agents with Cursor-specific fields
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("explorer.md"),
            "---\nname: explorer\ndescription: Explores the codebase\nis_background: false\nreadonly: true\nmodel: fast\n---\n\nYou explore and analyze code.\n",
        ).unwrap();

        // Hooks (Cursor format, camelCase)
        let hooks_config = serde_json::json!({
            "version": 1,
            "hooks": {
                "preToolUse": [{
                    "command": "./hooks/lint.sh",
                    "type": "command",
                    "timeout": 5
                }],
                "afterFileEdit": [{
                    "command": "./hooks/format.sh",
                    "type": "command"
                }]
            }
        });
        fs::write(
            root.join("hooks.json"),
            serde_json::to_string_pretty(&hooks_config).unwrap(),
        )
        .unwrap();

        // Rules (.mdc format)
        let rules_dir = root.join("rules");
        fs::create_dir_all(&rules_dir).unwrap();
        fs::write(
            rules_dir.join("typescript.mdc"),
            "---\nalwaysApply: true\ndescription: TypeScript conventions\n---\n\n# TypeScript\n\nUse strict TypeScript.\n",
        ).unwrap();
        fs::write(
            rules_dir.join("testing.mdc"),
            "---\nglobs: \"**/*.test.ts\"\nalwaysApply: false\ndescription: Testing guidelines\n---\n\n# Testing\n\nWrite thorough tests.\n",
        ).unwrap();

        // MCP
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "cursor-mcp": {
                    "command": "/usr/bin/cursor-mcp",
                    "args": ["--verbose"]
                }
            }
        });
        fs::write(
            root.join("mcp.json"),
            serde_json::to_string_pretty(&mcp_config).unwrap(),
        )
        .unwrap();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Claude → Cursor sync tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn claude_to_cursor_syncs_skills_with_frontmatter_stripping() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams {
        sync_skills: true,
        sync_commands: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_instructions: false,
        ..Default::default()
    };

    let report = orch.sync(&params).unwrap();
    assert!(report.success);
    assert!(report.skills.written >= 2);

    // Verify frontmatter was stripped
    let skill_path = setup.cursor_dir.path().join("skills/code-review/SKILL.md");
    assert!(skill_path.exists(), "Skill directory should be created");
    let content = fs::read_to_string(&skill_path).unwrap();
    assert!(!content.contains("---"), "Frontmatter should be stripped");
    assert!(
        content.contains("# Code Review"),
        "Body should be preserved"
    );

    // Verify module files preserved
    let module_path = setup
        .cursor_dir
        .path()
        .join("skills/code-review/modules/checklist.md");
    assert!(module_path.exists(), "Module files should be preserved");
}

#[test]
fn claude_to_cursor_syncs_commands() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams {
        sync_commands: true,
        sync_skills: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_instructions: false,
        ..Default::default()
    };

    let report = orch.sync(&params).unwrap();
    assert!(report.success);
    assert_eq!(report.commands.written, 2);

    // Verify command files exist
    let cmd_path = setup.cursor_dir.path().join("commands/commit-msg.md");
    assert!(cmd_path.exists());
    let content = fs::read_to_string(&cmd_path).unwrap();
    assert!(content.contains("Commit Message"));
}

#[test]
fn claude_to_cursor_translates_agent_frontmatter() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams {
        sync_agents: true,
        sync_skills: false,
        sync_commands: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_instructions: false,
        ..Default::default()
    };

    let report = orch.sync(&params).unwrap();
    assert!(report.success);
    assert!(report.agents.written >= 2);

    // Verify agent field translation
    let agent_path = setup.cursor_dir.path().join("agents/reviewer.md");
    assert!(agent_path.exists());
    let content = fs::read_to_string(&agent_path).unwrap();
    assert!(
        content.contains("is_background: true"),
        "background should become is_background"
    );
    assert!(
        !content.lines().any(|l| l.trim().starts_with("tools:")),
        "tools should be stripped"
    );
    assert!(
        !content.lines().any(|l| l.trim().starts_with("isolation:")),
        "isolation should be stripped"
    );
}

#[test]
fn claude_to_cursor_generates_mdc_rules() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams {
        sync_instructions: true,
        sync_skills: false,
        sync_commands: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        ..Default::default()
    };

    let report = orch.sync(&params).unwrap();
    assert!(report.success);
    assert!(report.instructions.written >= 1);

    // CLAUDE.md should become an always-apply rule
    let rules_dir = setup.cursor_dir.path().join("rules");
    assert!(rules_dir.exists(), "Rules directory should be created");

    let mut found_always_apply = false;
    for entry in fs::read_dir(&rules_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "mdc") {
            let content = fs::read_to_string(&path).unwrap();
            if content.contains("alwaysApply: true") {
                found_always_apply = true;
            }
        }
    }
    assert!(
        found_always_apply,
        "CLAUDE.md should become an alwaysApply rule"
    );
}

#[test]
fn claude_to_cursor_syncs_mcp() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams {
        sync_mcp_servers: true,
        sync_skills: false,
        sync_commands: false,
        sync_preferences: false,
        sync_agents: false,
        sync_instructions: false,
        ..Default::default()
    };

    let report = orch.sync(&params).unwrap();
    assert!(report.success);
    assert!(report.mcp_servers.written >= 1);

    let mcp_path = setup.cursor_dir.path().join("mcp.json");
    assert!(mcp_path.exists());
    let content = fs::read_to_string(&mcp_path).unwrap();
    assert!(content.contains("skrills"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Cursor → Claude sync tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cursor_to_claude_syncs_skills() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_cursor_config();

    let source = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let target = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams {
        sync_skills: true,
        sync_commands: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_instructions: false,
        ..Default::default()
    };

    let report = orch.sync(&params).unwrap();
    assert!(report.success);
    assert!(report.skills.written >= 1);

    let skill_path = setup.claude_dir.path().join("skills/test-runner/SKILL.md");
    assert!(skill_path.exists());
}

#[test]
fn cursor_to_claude_syncs_hooks_with_event_translation() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_cursor_config();

    let source = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let hooks = source.read_hooks().unwrap();

    // Should read both preToolUse and afterFileEdit events
    assert!(hooks.len() >= 2, "Should read Cursor hook events");

    // Cursor camelCase events are translated to Claude PascalCase where a mapping exists
    let event_names: Vec<&str> = hooks.iter().map(|h| h.name.as_str()).collect();
    assert!(
        event_names.contains(&"PreToolUse"),
        "preToolUse should be translated to PreToolUse"
    );
    // Cursor-only events without a Claude equivalent are preserved as-is
    assert!(
        event_names.contains(&"afterFileEdit"),
        "Cursor-only events should be preserved"
    );
}

#[test]
fn cursor_to_claude_syncs_rules_as_instructions() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_cursor_config();

    let source = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let instructions = source.read_instructions().unwrap();

    assert!(instructions.len() >= 2, "Should read .mdc rules");

    let names: Vec<&str> = instructions.iter().map(|i| i.name.as_str()).collect();
    assert!(names.contains(&"typescript"), "Should find typescript rule");
    assert!(names.contains(&"testing"), "Should find testing rule");
}

// ─────────────────────────────────────────────────────────────────────────────
// Round-trip fidelity tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn round_trip_commands_claude_to_cursor_to_claude() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let claude_source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let original_commands = claude_source.read_commands(false).unwrap();

    // Claude → Cursor
    let cursor_target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch1 = SyncOrchestrator::new(claude_source, cursor_target);
    let params = SyncParams {
        sync_commands: true,
        sync_skills: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_instructions: false,
        ..Default::default()
    };
    let _ = orch1.sync(&params).unwrap();

    // Cursor → Claude (to a new dir)
    let round_trip_dir = TempDir::new().unwrap();
    let cursor_source = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let claude_target = ClaudeAdapter::with_root(round_trip_dir.path().to_path_buf());
    let orch2 = SyncOrchestrator::new(cursor_source, claude_target);
    let _ = orch2.sync(&params).unwrap();

    // Compare
    let round_tripped = ClaudeAdapter::with_root(round_trip_dir.path().to_path_buf())
        .read_commands(false)
        .unwrap();

    assert_eq!(
        original_commands.len(),
        round_tripped.len(),
        "Command count should match"
    );
    for (orig, rt) in original_commands.iter().zip(round_tripped.iter()) {
        assert_eq!(orig.name, rt.name, "Command names should match");
        assert_eq!(orig.content, rt.content, "Command content should match");
    }
}

#[test]
fn round_trip_mcp_claude_to_cursor_to_claude() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let claude_source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let original_mcp = claude_source.read_mcp_servers().unwrap();

    // Claude → Cursor
    let cursor_target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch1 = SyncOrchestrator::new(claude_source, cursor_target);
    let params = SyncParams {
        sync_mcp_servers: true,
        sync_skills: false,
        sync_commands: false,
        sync_preferences: false,
        sync_agents: false,
        sync_instructions: false,
        ..Default::default()
    };
    let _ = orch1.sync(&params).unwrap();

    // Cursor → Claude
    let round_trip_dir = TempDir::new().unwrap();
    let cursor_source = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let claude_target = ClaudeAdapter::with_root(round_trip_dir.path().to_path_buf());
    let orch2 = SyncOrchestrator::new(cursor_source, claude_target);
    let _ = orch2.sync(&params).unwrap();

    let round_tripped = ClaudeAdapter::with_root(round_trip_dir.path().to_path_buf())
        .read_mcp_servers()
        .unwrap();

    assert_eq!(
        original_mcp.len(),
        round_tripped.len(),
        "MCP server count should match"
    );
    for (name, orig) in &original_mcp {
        let rt = round_tripped
            .get(name)
            .expect("MCP server should exist after round-trip");
        assert_eq!(orig.command, rt.command, "MCP command should match");
        assert_eq!(orig.args, rt.args, "MCP args should match");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Full sync (all artifact types)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn full_claude_to_cursor_sync() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams::default();
    let report = orch.sync(&params).unwrap();

    assert!(report.success, "Full sync should succeed");
    assert!(
        report.skills.written >= 2,
        "Should sync skills: {}",
        report.skills.written
    );
    assert!(
        report.commands.written >= 2,
        "Should sync commands: {}",
        report.commands.written
    );
    assert!(
        report.agents.written >= 2,
        "Should sync agents: {}",
        report.agents.written
    );
    assert!(
        report.mcp_servers.written >= 1,
        "Should sync MCP: {}",
        report.mcp_servers.written
    );
    assert!(
        report.instructions.written >= 1,
        "Should sync instructions: {}",
        report.instructions.written
    );

    // Verify directory structure created
    let cursor_root = setup.cursor_dir.path();
    assert!(cursor_root.join("skills").exists());
    assert!(cursor_root.join("commands").exists());
    assert!(cursor_root.join("agents").exists());
    assert!(cursor_root.join("rules").exists());
    assert!(cursor_root.join("mcp.json").exists());
}

#[test]
fn full_cursor_to_claude_sync() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_cursor_config();

    let source = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let target = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams::default();
    let report = orch.sync(&params).unwrap();

    assert!(report.success, "Full sync should succeed");
    assert!(report.skills.written >= 1, "Should sync skills");
    assert!(report.commands.written >= 1, "Should sync commands");
    assert!(report.agents.written >= 1, "Should sync agents");

    // Verify directory structure created
    let claude_root = setup.claude_dir.path();
    assert!(claude_root.join("skills").exists());
    assert!(claude_root.join("commands").exists());
    assert!(claude_root.join("agents").exists());
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge cases
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn sync_to_cursor_with_empty_source() {
    let setup = CursorSyncTestSetup::new().unwrap();
    // Don't set up any config — empty source

    let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams::default();
    let report = orch.sync(&params).unwrap();

    assert!(report.success, "Sync with empty source should succeed");
    assert_eq!(report.skills.written, 0);
    assert_eq!(report.commands.written, 0);
}

#[test]
fn dry_run_does_not_write_files() {
    let setup = CursorSyncTestSetup::new().unwrap();
    setup.setup_claude_config();

    let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
    let target = CursorAdapter::with_root(setup.cursor_dir.path().to_path_buf());
    let orch = SyncOrchestrator::new(source, target);

    let params = SyncParams {
        dry_run: true,
        ..Default::default()
    };
    let report = orch.sync(&params).unwrap();

    assert!(report.success);
    // Dry run reports counts but doesn't create files
    assert!(
        !setup.cursor_dir.path().join("skills").exists(),
        "Dry run should not create directories"
    );
}

#[test]
fn create_adapter_factory_works() {
    use skrills_sync::create_adapter;

    // Valid platforms
    assert!(create_adapter("cursor").is_ok());
    assert_eq!(create_adapter("cursor").unwrap().name(), "cursor");

    // Invalid platform
    assert!(create_adapter("vscode").is_err());
}

#[test]
fn is_valid_platform_recognizes_all() {
    use skrills_sync::is_valid_platform;

    assert!(is_valid_platform("claude"));
    assert!(is_valid_platform("codex"));
    assert!(is_valid_platform("copilot"));
    assert!(is_valid_platform("cursor"));
    assert!(!is_valid_platform("vscode"));
    assert!(!is_valid_platform(""));
}
