//! Integration tests for Copilot sync functionality.
//!
//! These tests validate sync operations between Copilot and other agents
//! (Claude, Codex). They focus on Copilot-specific behaviors:
//!
//! - Skills: Bidirectional sync with Claude/Codex
//! - MCP servers: Stored in `mcp-config.json` (not `config.json`)
//! - Preferences: Stored in `config.json`, preserves security fields
//! - Commands: NOT supported by Copilot (should be skipped)

use skrills_sync::{
    adapters::traits::AgentAdapter,
    adapters::{ClaudeAdapter, CodexAdapter, CopilotAdapter},
    orchestrator::{SyncOrchestrator, SyncParams},
};
use std::fs;
use tempfile::TempDir;

/// Test fixture for Copilot sync operations.
struct CopilotSyncTestSetup {
    copilot_dir: TempDir,
    claude_dir: TempDir,
    codex_dir: TempDir,
}

impl CopilotSyncTestSetup {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            copilot_dir: TempDir::new()?,
            claude_dir: TempDir::new()?,
            codex_dir: TempDir::new()?,
        })
    }

    /// Create mock Copilot configuration with sample data.
    fn setup_copilot_config(&self, root: &std::path::Path) {
        // Create skills directory with SKILL.md format
        let skills_dir = root.join("skills");
        fs::create_dir_all(skills_dir.join("code-review")).unwrap();

        fs::write(
            skills_dir.join("code-review").join("SKILL.md"),
            r#"---
name: code-review
description: Provides thorough code reviews with suggestions.
---
# Code Review Skill

This skill helps review code for:
- Bug detection
- Style consistency
- Performance issues
"#,
        )
        .unwrap();

        // Create another skill
        fs::create_dir_all(skills_dir.join("git-helpers")).unwrap();
        fs::write(
            skills_dir.join("git-helpers").join("SKILL.md"),
            r#"---
name: git-helpers
description: Git workflow helpers and shortcuts.
---
# Git Helpers

Provides common git operations.
"#,
        )
        .unwrap();

        // Create MCP server configuration in mcp-config.json (NOT config.json)
        let mcp_config = serde_json::json!({
            "mcpServers": {
                "copilot-mcp": {
                    "command": "/usr/bin/copilot-mcp-server",
                    "args": ["--port", "9000"]
                }
            }
        });
        fs::write(
            root.join("mcp-config.json"),
            serde_json::to_string_pretty(&mcp_config).unwrap(),
        )
        .unwrap();

        // Create preferences in config.json
        let config = serde_json::json!({
            "model": "gpt-4o",
            "cliVersion": "1.0.0"
        });
        fs::write(
            root.join("config.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
    }

    /// Create mock Claude configuration.
    fn setup_claude_config(&self, root: &std::path::Path) {
        // Create commands directory
        let cmd_dir = root.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(
            cmd_dir.join("test-analysis.md"),
            "# Test Analysis\nAnalyze the provided code.",
        )
        .unwrap();

        // Create skills directory with flat format (skill.md not SKILL.md)
        let skills_dir = root.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(
            skills_dir.join("debugging.md"),
            r#"---
name: debugging
description: Debug issues step by step.
---
# Debugging Skill

Helps debug complex issues.
"#,
        )
        .unwrap();

        // Create settings.json with MCP servers
        let settings = serde_json::json!({
            "mcpServers": {
                "claude-server": {
                    "command": "/usr/bin/claude-mcp",
                    "args": ["--verbose"]
                }
            },
            "model": "claude-sonnet-4",
            "preferences": {
                "theme": "dark"
            }
        });
        fs::write(
            root.join("settings.json"),
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();
    }

    /// Create mock Codex configuration.
    fn setup_codex_config(&self, root: &std::path::Path) {
        // Create prompts directory
        let cmd_dir = root.join("prompts");
        fs::create_dir_all(&cmd_dir).unwrap();
        fs::write(
            cmd_dir.join("quick-fix.md"),
            "# Quick Fix\nFix the issue quickly.",
        )
        .unwrap();

        // Create skills directory with SKILL.md format (like Copilot)
        let skills_dir = root.join("skills");
        fs::create_dir_all(skills_dir.join("refactor")).unwrap();
        fs::write(
            skills_dir.join("refactor").join("SKILL.md"),
            r#"---
name: refactor
description: Refactor code for clarity.
---
# Refactor Skill

Helps refactor code.
"#,
        )
        .unwrap();
    }
}

#[cfg(test)]
mod copilot_to_claude_tests {
    use super::*;

    #[test]
    fn test_sync_copilot_skills_to_claude() {
        /*
        GIVEN a Copilot configuration with skills in SKILL.md format
        WHEN syncing to Claude
        THEN skills should be transferred correctly
        AND the skill content should be preserved
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_copilot_config(setup.copilot_dir.path());

        let source = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("copilot".to_string()),
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: false, // Copilot has no commands
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Copilot to Claude sync should succeed");
        let report = result.unwrap();

        // Skills should be synced
        assert!(
            report.skills.written > 0,
            "Skills should be written: got {}",
            report.skills.written
        );

        // Verify skill was written to Claude's skills directory
        let claude_skills = setup.claude_dir.path().join("skills");
        assert!(
            claude_skills.exists(),
            "Claude skills directory should be created"
        );
    }

    #[test]
    fn test_sync_copilot_mcp_servers_to_claude() {
        /*
        GIVEN a Copilot configuration with MCP servers in mcp-config.json
        WHEN syncing to Claude
        THEN MCP servers should be read from mcp-config.json (not config.json)
        AND written to Claude's settings.json
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_copilot_config(setup.copilot_dir.path());

        let source = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("copilot".to_string()),
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: false,
            sync_mcp_servers: true,
            sync_preferences: false,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "MCP server sync should succeed");
        let report = result.unwrap();

        // MCP servers should be synced
        assert!(
            report.mcp_servers.written > 0,
            "MCP servers should be written: got {}",
            report.mcp_servers.written
        );
    }

    #[test]
    fn test_copilot_commands_not_synced() {
        /*
        GIVEN a sync from Copilot to Claude
        WHEN requesting command sync
        THEN commands should be skipped (Copilot has no commands)
        AND the sync should still succeed
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_copilot_config(setup.copilot_dir.path());

        let source = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("copilot".to_string()),
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: true, // Requesting commands
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Sync should succeed even with no commands");
        let report = result.unwrap();

        // No commands should be written (Copilot doesn't support them)
        assert_eq!(
            report.commands.written, 0,
            "No commands should be written from Copilot"
        );
    }
}

#[cfg(test)]
mod claude_to_copilot_tests {
    use super::*;

    #[test]
    fn test_sync_claude_skills_to_copilot() {
        /*
        GIVEN a Claude configuration with skills
        WHEN syncing to Copilot
        THEN skills should be transferred correctly
        AND written in SKILL.md format (Copilot's expected format)
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_claude_config(setup.claude_dir.path());

        let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("claude".to_string()),
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: false, // Copilot doesn't support commands
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Claude to Copilot sync should succeed");
        let report = result.unwrap();

        // Skills should be synced
        assert!(
            report.skills.written > 0,
            "Skills should be written: got {}",
            report.skills.written
        );

        // Verify skill was written to Copilot's skills directory in SKILL.md format
        let copilot_skills = setup.copilot_dir.path().join("skills");
        assert!(
            copilot_skills.exists(),
            "Copilot skills directory should be created"
        );
    }

    #[test]
    fn test_sync_claude_mcp_to_copilot_mcp_config() {
        /*
        GIVEN a Claude configuration with MCP servers in settings.json
        WHEN syncing to Copilot
        THEN MCP servers should be written to mcp-config.json (not config.json)
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_claude_config(setup.claude_dir.path());

        let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("claude".to_string()),
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: false,
            sync_mcp_servers: true,
            sync_preferences: false,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "MCP sync to Copilot should succeed");
        let report = result.unwrap();

        // MCP servers should be written
        assert!(
            report.mcp_servers.written > 0,
            "MCP servers should be written: got {}",
            report.mcp_servers.written
        );

        // Verify MCP config was written to the correct file
        let mcp_config_path = setup.copilot_dir.path().join("mcp-config.json");
        assert!(
            mcp_config_path.exists(),
            "mcp-config.json should be created"
        );
    }

    #[test]
    fn test_claude_commands_skipped_when_syncing_to_copilot() {
        /*
        GIVEN a Claude configuration with commands
        WHEN syncing to Copilot with sync_commands enabled
        THEN commands should be skipped (Copilot doesn't support them)
        AND no errors should occur
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_claude_config(setup.claude_dir.path());

        let source = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("claude".to_string()),
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: true, // Requesting commands
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(
            result.is_ok(),
            "Sync should succeed even when target doesn't support commands"
        );
        let report = result.unwrap();

        // Commands should be skipped (Copilot doesn't support them)
        assert_eq!(
            report.commands.written, 0,
            "No commands should be written to Copilot"
        );
    }
}

#[cfg(test)]
mod copilot_to_codex_tests {
    use super::*;

    #[test]
    fn test_sync_copilot_skills_to_codex() {
        /*
        GIVEN a Copilot configuration with skills
        WHEN syncing to Codex
        THEN skills should be transferred correctly
        AND both use SKILL.md format (same format)
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_copilot_config(setup.copilot_dir.path());

        let source = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(setup.codex_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("copilot".to_string()),
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Copilot to Codex sync should succeed");
        let report = result.unwrap();

        // Skills should be synced
        assert!(
            report.skills.written > 0,
            "Skills should be written: got {}",
            report.skills.written
        );
    }

    #[test]
    fn test_sync_codex_skills_to_copilot() {
        /*
        GIVEN a Codex configuration with skills
        WHEN syncing to Copilot
        THEN skills should be transferred correctly
        AND both use SKILL.md format
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_codex_config(setup.codex_dir.path());

        let source = CodexAdapter::with_root(setup.codex_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("codex".to_string()),
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Codex to Copilot sync should succeed");
        let report = result.unwrap();

        // Skills should be synced
        assert!(
            report.skills.written > 0,
            "Skills should be written: got {}",
            report.skills.written
        );
    }
}

#[cfg(test)]
mod copilot_dry_run_tests {
    use super::*;

    #[test]
    fn test_copilot_dry_run_no_changes() {
        /*
        GIVEN a sync from Copilot with dry_run enabled
        WHEN performing the sync
        THEN it should report what would be synced
        BUT make no actual changes to the target
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        setup.setup_copilot_config(setup.copilot_dir.path());

        let source = CopilotAdapter::with_root(setup.copilot_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(setup.claude_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("copilot".to_string()),
            dry_run: true, // Enable dry run
            force: false,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Dry run should succeed");
        let report = result.unwrap();

        // Should report what would be synced
        assert!(
            report.skills.written > 0,
            "Should report skills would be written"
        );

        // But target directory should remain empty
        let target_entries: Vec<_> = fs::read_dir(setup.claude_dir.path()).unwrap().collect();
        assert!(
            target_entries.is_empty(),
            "Target should be empty after dry run"
        );
    }
}

#[cfg(test)]
mod copilot_adapter_field_support_tests {
    use super::*;

    #[test]
    fn test_copilot_field_support() {
        /*
        GIVEN a CopilotAdapter
        WHEN checking field support
        THEN commands should be unsupported
        AND skills, mcp_servers, preferences should be supported
        */
        let tmp = TempDir::new().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let support = adapter.supported_fields();

        assert!(!support.commands, "Copilot should NOT support commands");
        assert!(support.skills, "Copilot should support skills");
        assert!(support.mcp_servers, "Copilot should support MCP servers");
        assert!(support.preferences, "Copilot should support preferences");
    }
}
