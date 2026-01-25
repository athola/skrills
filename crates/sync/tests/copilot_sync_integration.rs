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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
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
    fn test_claude_commands_synced_to_copilot_as_prompts() {
        /*
        GIVEN a Claude configuration with commands
        WHEN syncing to Copilot with sync_commands enabled
        THEN commands should be written as *.prompts.md files
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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Sync should succeed");
        let report = result.unwrap();

        // Commands should now be written as prompts
        assert!(
            report.commands.written > 0,
            "Commands should be written to Copilot as prompts"
        );

        // Verify the prompts directory was created with .prompts.md files
        let prompts_dir = setup.copilot_dir.path().join("prompts");
        assert!(prompts_dir.exists(), "Prompts directory should be created");
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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
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
            sync_agents: false,
            sync_instructions: true,
            skip_existing_instructions: false,
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
        THEN commands should NOT be supported (prompts differ from Claude commands/Codex prompts)
        AND skills, mcp_servers, preferences should be supported
        */
        let tmp = TempDir::new().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let support = adapter.supported_fields();

        // Copilot prompts are detailed instruction files, NOT equivalent to
        // Claude commands/Codex prompts (which are quick atomic shortcuts)
        assert!(
            !support.commands,
            "Copilot should NOT support commands (prompts are different)"
        );
        assert!(support.skills, "Copilot should support skills");
        assert!(support.mcp_servers, "Copilot should support MCP servers");
        assert!(support.preferences, "Copilot should support preferences");
    }
}

#[cfg(test)]
mod copilot_security_tests {
    use super::*;
    use skrills_sync::common::Command;
    use std::path::PathBuf;
    use std::time::SystemTime;

    #[test]
    fn test_path_traversal_blocked_during_sync() {
        /*
        GIVEN a skill with a path traversal attack in its name
        WHEN syncing to Copilot
        THEN the traversal should be blocked
        AND the skill should be written to a safe location
        */
        let tmp = TempDir::new().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        // Create a malicious skill name with path traversal
        let content = b"---\nname: malicious\ndescription: Malicious skill\n---\nContent".to_vec();
        let malicious_skill = Command {
            name: "../../../etc/passwd".to_string(),
            content: content.clone(),
            source_path: PathBuf::from("/fake/path/skill.md"),
            modified: SystemTime::now(),
            hash: "abc123".to_string(),
            modules: Vec::new(),
        };

        // Write the skill
        let result = adapter.write_skills(&[malicious_skill]);
        assert!(
            result.is_ok(),
            "Write should succeed (traversal is sanitized)"
        );

        // Verify the skill was written to a SAFE location, not /etc/passwd
        // The sanitize_name function should have stripped the traversal
        let skills_dir = tmp.path().join("skills");
        assert!(skills_dir.exists(), "Skills directory should exist");

        // The sanitized name should be "etc/passwd" (dots and slashes stripped from leading segments)
        let safe_path = skills_dir.join("etc/passwd").join("SKILL.md");
        assert!(
            safe_path.exists(),
            "Skill should be at safe path {:?}, not at system /etc/passwd",
            safe_path
        );

        // Additional check: the skill should NOT have been written outside tmp
        let written_content = fs::read_to_string(&safe_path).unwrap();
        assert!(
            written_content.contains("Malicious skill"),
            "Skill content should be at safe location"
        );
    }

    #[test]
    fn test_nested_skill_paths_preserved() {
        /*
        GIVEN a skill with a legitimate nested path (category/skill-name)
        WHEN syncing to Copilot
        THEN the nested path structure should be preserved
        */
        let tmp = TempDir::new().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let nested_skill = Command {
            name: "category/my-skill".to_string(),
            content: b"---\nname: my-skill\ndescription: Nested skill\n---\nContent".to_vec(),
            source_path: PathBuf::from("/fake/path/category/my-skill/skill.md"),
            modified: SystemTime::now(),
            hash: "def456".to_string(),
            modules: Vec::new(),
        };

        let result = adapter.write_skills(&[nested_skill]);
        assert!(result.is_ok(), "Write should succeed");

        // Verify nested structure is preserved
        let nested_path = tmp.path().join("skills/category/my-skill/SKILL.md");
        assert!(
            nested_path.exists(),
            "Nested skill should be at {:?}",
            nested_path
        );
    }

    #[test]
    fn test_mixed_traversal_and_nested_paths() {
        /*
        GIVEN a skill with mixed traversal attempts and legitimate nesting
        WHEN syncing to Copilot
        THEN traversal segments should be removed but legitimate paths preserved
        */
        let tmp = TempDir::new().unwrap();
        let adapter = CopilotAdapter::with_root(tmp.path().to_path_buf());

        let mixed_skill = Command {
            name: "category/../other/./skill".to_string(),
            content: b"---\nname: skill\ndescription: Mixed path skill\n---\nContent".to_vec(),
            source_path: PathBuf::from("/fake/path/skill.md"),
            modified: SystemTime::now(),
            hash: "ghi789".to_string(),
            modules: Vec::new(),
        };

        let result = adapter.write_skills(&[mixed_skill]);
        assert!(result.is_ok(), "Write should succeed");

        // The sanitized path should be "category/other/skill" (traversal removed)
        let safe_path = tmp.path().join("skills/category/other/skill/SKILL.md");
        assert!(
            safe_path.exists(),
            "Mixed path skill should be at safe location {:?}",
            safe_path
        );
    }
}

// ==========================================
// Agent Sync Integration Tests
// ==========================================

mod agent_sync_tests {
    use super::*;

    fn setup_claude_with_agents(root: &std::path::Path) {
        // Create agents directory
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        // Create a Claude agent
        fs::write(
            agents_dir.join("code-review-agent.md"),
            r#"---
name: code-review-agent
description: Performs thorough code reviews
model: opus
color: blue
---
# Code Review Agent

This agent specializes in reviewing code for:
- Bug detection
- Security vulnerabilities
- Performance issues
"#,
        )
        .unwrap();

        // Create another agent
        fs::write(
            agents_dir.join("refactor-agent.md"),
            r#"---
name: refactor-agent
description: Helps refactor code
model: sonnet
---
# Refactor Agent

This agent helps with code refactoring tasks.
"#,
        )
        .unwrap();
    }

    fn setup_copilot_with_agents(root: &std::path::Path) {
        // Create agents directory
        let agents_dir = root.join("agents");
        fs::create_dir_all(&agents_dir).unwrap();

        // Create a Copilot agent
        fs::write(
            agents_dir.join("test-agent.agent.md"),
            r#"---
name: test-agent
description: A test agent for Copilot
target: github-copilot
---
# Test Agent

This is a test agent for Copilot.
"#,
        )
        .unwrap();
    }

    #[test]
    fn test_sync_claude_agents_to_copilot() {
        /*
        GIVEN a Claude configuration with agents
        WHEN syncing to Copilot with sync_agents enabled
        THEN agents should be transferred correctly
        AND Claude-specific fields (model, color) should be transformed
        AND target: github-copilot should be added
        */
        let claude_dir = TempDir::new().unwrap();
        let copilot_dir = TempDir::new().unwrap();

        setup_claude_with_agents(claude_dir.path());

        let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(copilot_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("claude".to_string()),
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: true, // Enable agent sync
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(
            result.is_ok(),
            "Claude to Copilot agent sync should succeed"
        );
        let report = result.unwrap();

        // Agents should be synced
        assert!(
            report.agents.written > 0,
            "Agents should be written: got {}",
            report.agents.written
        );

        // Verify agent was written to Copilot's agents directory
        let copilot_agents = copilot_dir.path().join("agents");
        assert!(
            copilot_agents.exists(),
            "Copilot agents directory should be created"
        );

        // Verify agent file exists with correct naming
        let agent_path = copilot_agents.join("code-review-agent.agent.md");
        assert!(agent_path.exists(), "Agent should be at {:?}", agent_path);

        // Verify content transformation
        let content = fs::read_to_string(&agent_path).unwrap();
        assert!(
            content.contains("target: github-copilot"),
            "Should have target: github-copilot"
        );
        assert!(
            !content.contains("model: opus"),
            "Should NOT have model: opus (Claude-specific)"
        );
        assert!(
            !content.contains("color: blue"),
            "Should NOT have color: blue (Claude-specific)"
        );
    }

    #[test]
    fn test_sync_copilot_agents_to_claude() {
        /*
        GIVEN a Copilot configuration with agents
        WHEN syncing to Claude with sync_agents enabled
        THEN agents should be transferred correctly
        */
        let claude_dir = TempDir::new().unwrap();
        let copilot_dir = TempDir::new().unwrap();

        setup_copilot_with_agents(copilot_dir.path());

        let source = CopilotAdapter::with_root(copilot_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("copilot".to_string()),
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: true,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(
            result.is_ok(),
            "Copilot to Claude agent sync should succeed"
        );
        let report = result.unwrap();

        assert!(
            report.agents.written > 0,
            "Agents should be written: got {}",
            report.agents.written
        );

        // Verify agent was written to Claude's agents directory
        let claude_agents = claude_dir.path().join("agents");
        assert!(
            claude_agents.exists(),
            "Claude agents directory should be created"
        );

        let agent_path = claude_agents.join("test-agent.md");
        assert!(agent_path.exists(), "Agent should be at {:?}", agent_path);
    }

    #[test]
    fn test_sync_agents_disabled_by_default_in_existing_tests() {
        /*
        GIVEN existing SyncParams with sync_agents: false
        WHEN syncing with agents in source
        THEN agents should NOT be synced
        */
        let claude_dir = TempDir::new().unwrap();
        let copilot_dir = TempDir::new().unwrap();

        setup_claude_with_agents(claude_dir.path());

        let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(copilot_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("claude".to_string()),
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: false, // Explicitly disabled
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Sync should succeed");
        let report = result.unwrap();

        // Agents should NOT be synced
        assert_eq!(
            report.agents.written, 0,
            "No agents should be written when sync_agents is false"
        );

        // Verify no agents directory was created
        let copilot_agents = copilot_dir.path().join("agents");
        assert!(
            !copilot_agents.exists(),
            "Copilot agents directory should NOT be created"
        );
    }

    #[test]
    fn test_agents_dry_run() {
        /*
        GIVEN agents to sync with dry_run enabled
        WHEN performing the sync
        THEN it should report what would be synced
        BUT make no actual changes
        */
        let claude_dir = TempDir::new().unwrap();
        let copilot_dir = TempDir::new().unwrap();

        setup_claude_with_agents(claude_dir.path());

        let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
        let target = CopilotAdapter::with_root(copilot_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("claude".to_string()),
            dry_run: true, // Enable dry run
            force: false,
            sync_skills: false,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: true,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Dry run should succeed");
        let report = result.unwrap();

        // Should report what would be synced
        assert!(
            report.agents.written > 0,
            "Should report agents would be written"
        );

        // But target directory should remain empty
        let copilot_agents = copilot_dir.path().join("agents");
        assert!(
            !copilot_agents.exists(),
            "Agents directory should NOT be created during dry run"
        );
    }

    #[test]
    fn test_sync_agents_codex_converts_to_skills() {
        /*
        GIVEN a sync from Claude to Codex
        WHEN syncing agents
        THEN agents should be converted to skills with "agent-" prefix
        (Codex doesn't have native agent support, so we convert them)
        */
        let claude_dir = TempDir::new().unwrap();
        let codex_dir = TempDir::new().unwrap();

        setup_claude_with_agents(claude_dir.path());

        let source = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(codex_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("claude".to_string()),
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: true, // Sync agents (will be converted to skills)
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Sync should succeed");
        let report = result.unwrap();

        // Agents should be written (converted to skills)
        assert_eq!(
            report.agents.written, 2,
            "Agents should be written to Codex as skills"
        );

        // Verify agents were written as skills with "agent-" prefix
        let skills_dir = codex_dir.path().join("skills");
        assert!(
            skills_dir.join("agent-code-review-agent/SKILL.md").exists(),
            "Agent should be written as skill with agent- prefix"
        );
        assert!(
            skills_dir.join("agent-refactor-agent/SKILL.md").exists(),
            "Agent should be written as skill with agent- prefix"
        );
    }

    #[test]
    fn test_sync_all_with_agents() {
        /*
        GIVEN a full sync including agents
        WHEN syncing from Claude to Copilot
        THEN all supported fields should be synced including agents
        */
        let setup = CopilotSyncTestSetup::new().unwrap();

        // Set up Claude with skills, MCP, and agents
        setup.setup_claude_config(setup.claude_dir.path());
        setup_claude_with_agents(setup.claude_dir.path());

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
            sync_agents: true,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        assert!(result.is_ok(), "Full sync should succeed");
        let report = result.unwrap();

        // All fields should be synced
        assert!(report.skills.written > 0, "Skills should be synced");
        assert!(
            report.mcp_servers.written > 0,
            "MCP servers should be synced"
        );
        assert!(report.agents.written > 0, "Agents should be synced");
    }
}

// ==========================================
// Plugins Cache Sync Tests
// ==========================================

mod plugins_cache_sync_tests {
    use super::*;

    #[test]
    fn test_sync_claude_plugins_cache_to_copilot() {
        /*
        GIVEN a Claude installation with a skill in plugins/cache/
        WHEN syncing to Copilot
        THEN the skill should be written to ~/.copilot/skills/<skill-name>/SKILL.md
             without preserving the plugins/cache/ path structure
        */
        let claude_dir = TempDir::new().unwrap();
        let copilot_dir = TempDir::new().unwrap();

        // Create a skill in Claude's plugins cache
        // ~/.claude/plugins/cache/claude-night-market/abstract/1.2.0/skills/hooks-eval/SKILL.md
        let cache_skill_dir = claude_dir
            .path()
            .join("plugins/cache/claude-night-market/abstract/1.2.0/skills/hooks-eval");
        fs::create_dir_all(&cache_skill_dir).unwrap();
        fs::write(
            cache_skill_dir.join("SKILL.md"),
            r#"---
name: hooks-eval
description: Evaluate hooks for quality
---
# Hooks Eval

This skill evaluates hook implementations.
"#,
        )
        .unwrap();

        // Sync from Claude to Copilot
        let claude = ClaudeAdapter::with_root(claude_dir.path().to_path_buf());
        let copilot = CopilotAdapter::with_root(copilot_dir.path().to_path_buf());
        let orchestrator = SyncOrchestrator::new(claude, copilot);

        let result = orchestrator.sync(&SyncParams::default());
        assert!(result.is_ok(), "Sync should succeed: {:?}", result.err());

        // Verify the skill was written to the correct location
        // Should be ~/.copilot/skills/hooks-eval/SKILL.md
        // NOT ~/.copilot/skills/plugins/cache/.../SKILL.md
        let expected_path = copilot_dir.path().join("skills/hooks-eval/SKILL.md");
        assert!(
            expected_path.exists(),
            "Skill should be at {:?}, not with plugins/cache/ prefix",
            expected_path
        );

        // Verify NO plugins/cache directory was created
        let wrong_path = copilot_dir.path().join("skills/plugins");
        assert!(
            !wrong_path.exists(),
            "Should not create plugins/ directory in skills: {:?}",
            wrong_path
        );

        // Verify content is correct
        let content = fs::read_to_string(&expected_path).unwrap();
        assert!(content.contains("hooks-eval"));
        assert!(content.contains("Evaluate hooks for quality"));
    }
}
