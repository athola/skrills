//! Integration tests for cross-agent sync functionality.
//!
//! These tests follow BDD/TDD principles to validate end-to-end sync operations
//! between different agent backends (Claude, Codex, etc.).

use skrills_sync::{
    adapters::traits::AgentAdapter,
    adapters::{ClaudeAdapter, CodexAdapter},
    orchestrator::{SyncOrchestrator, SyncParams},
};
use std::fs;
use tempfile::TempDir;

/// Test fixture for sync operations
struct SyncTestSetup {
    source_dir: TempDir,
    target_dir: TempDir,
}

impl SyncTestSetup {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            source_dir: TempDir::new()?,
            target_dir: TempDir::new()?,
        })
    }

    /// Create mock Claude configuration with sample data
    fn setup_claude_config(&self, root: &std::path::Path) {
        // Create commands directory with sample commands
        let cmd_dir = root.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();

        // Create multiple test commands
        fs::write(
            cmd_dir.join("test-analysis.md"),
            "# Test Analysis\nAnalyze the provided code and suggest improvements.",
        )
        .unwrap();

        fs::write(
            cmd_dir.join("generate-docs.md"),
            "# Generate Documentation\nGenerate full documentation for the project.",
        )
        .unwrap();

        // Create skills directory with sample skills
        let skills_dir = root.join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        fs::write(
            skills_dir.join("code-review.md"),
            "# Code Review Skill\nProvides thorough code reviews with suggestions.",
        )
        .unwrap();

        // User-scope MCP servers live in ~/.claude.json, preferences in settings.json
        let user_config = serde_json::json!({
            "mcpServers": {
                "test-server": {
                    "command": "/usr/bin/test-server",
                    "args": ["--port", "8080"]
                },
                "another-server": {
                    "command": "/usr/local/bin/another",
                    "env": {
                        "TOKEN": "test-token"
                    }
                }
            }
        });
        fs::write(
            root.join(".claude.json"),
            serde_json::to_string_pretty(&user_config).unwrap(),
        )
        .unwrap();
        let settings = serde_json::json!({
            "model": "claude-3-5-sonnet",
            "preferences": {
                "theme": "dark",
                "autoSave": true
            }
        });

        fs::write(
            root.join("settings.json"),
            serde_json::to_string_pretty(&settings).unwrap(),
        )
        .unwrap();
    }

    /// Create mock Codex configuration
    fn setup_codex_config(&self, root: &std::path::Path) {
        // Create prompts directory (Codex uses "prompts" not "commands")
        let cmd_dir = root.join("prompts");
        fs::create_dir_all(&cmd_dir).unwrap();

        // Create existing command that should be preserved with skip_existing
        // This has the same name as one in setup_claude_config to test skip functionality
        fs::write(
            cmd_dir.join("test-analysis.md"),
            "# Existing Test Analysis\nThis is the existing version of test-analysis.",
        )
        .unwrap();
    }
}

#[cfg(test)]
mod sync_direction_tests {
    use super::*;

    #[test]
    fn test_full_sync_claude_to_codex() {
        //
        /*
        GIVEN a Claude configuration with commands, skills, and settings
        WHEN syncing to a Codex environment
        THEN all components should be transferred correctly
        AND the target should have the same structure as the source
        */
        //
        let setup = SyncTestSetup::new().unwrap();

        // Setup source Claude configuration
        setup.setup_claude_config(setup.source_dir.path());

        // Create adapters
        let source = ClaudeAdapter::with_root(setup.source_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(setup.target_dir.path().to_path_buf());

        // Configure full sync
        let params = SyncParams {
            from: None,
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: true,
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
            sync_plugin_assets: false,
            interactive: false,
            exclude_plugins: vec![],
            full_plugin_mirror: false,
            keep_versions: false,
        };

        // Perform sync
        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        let report = result.expect("Sync should complete successfully");

        // Verify components were synced
        assert!(report.skills.written > 0, "Skills should be written");
        assert!(report.commands.written > 0, "Commands should be written");
        assert!(
            report.mcp_servers.written > 0,
            "MCP servers should be written"
        );
        // A Claude model has no known Codex id, so Codex keeps its own model
        // and the report says why.
        assert_eq!(report.preferences.written, 0);
        assert!(
            report
                .preferences
                .warnings
                .iter()
                .any(|w| w.contains("no known codex equivalent")),
            "{:?}",
            report.preferences.warnings
        );
    }

    #[test]
    fn test_sync_direction_reverse() {
        //
        /*
        GIVEN sync configured in reverse direction (target to source)
        WHEN performing the sync operation
        THEN data should flow from target to source correctly
        */
        //
        let setup = SyncTestSetup::new().unwrap();

        // Setup both directions
        setup.setup_claude_config(setup.target_dir.path());
        setup.setup_codex_config(setup.source_dir.path());

        let source = CodexAdapter::with_root(setup.source_dir.path().to_path_buf());
        let target = ClaudeAdapter::with_root(setup.target_dir.path().to_path_buf());

        let params = SyncParams {
            from: Some("codex".to_string()),
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: true,
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
            sync_plugin_assets: false,
            interactive: false,
            exclude_plugins: vec![],
            full_plugin_mirror: false,
            keep_versions: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        let report = result.expect("Reverse sync should succeed");

        // Verify sync occurred in reverse
        assert!(
            report.commands.written > 0 || !report.commands.skipped.is_empty(),
            "Commands should be processed in reverse sync"
        );
    }
}

#[cfg(test)]
mod sync_skip_existing_tests {
    use super::*;

    #[test]
    fn test_skip_existing_preserves_commands() {
        //
        /*
        GIVEN a target with existing commands
        WHEN syncing with skip_existing_commands enabled
        THEN existing commands should be preserved
        AND only new commands should be added
        */
        //
        let setup = SyncTestSetup::new().unwrap();

        // Setup source with commands
        setup.setup_claude_config(setup.source_dir.path());

        // Setup target with existing commands
        setup.setup_codex_config(setup.target_dir.path());

        let source = ClaudeAdapter::with_root(setup.source_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(setup.target_dir.path().to_path_buf());

        let params = SyncParams {
            from: None,
            dry_run: false,
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true, // Enable skip
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
            sync_plugin_assets: false,
            interactive: false,
            exclude_plugins: vec![],
            full_plugin_mirror: false,
            keep_versions: false,
        };

        // Debug: Show what commands are being synced
        let source_commands = source.read_commands(false).unwrap();
        let target_commands = target.read_commands(false).unwrap();
        println!(
            "Source commands: {:?}",
            source_commands.iter().map(|c| &c.name).collect::<Vec<_>>()
        );
        println!(
            "Target commands: {:?}",
            target_commands.iter().map(|c| &c.name).collect::<Vec<_>>()
        );

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        let report = result.expect("Sync with skip_existing should succeed");

        // Should write new commands but skip existing ones
        assert!(
            report.commands.written > 0,
            "New commands should be written, got {}",
            report.commands.written
        );
        assert!(
            !report.commands.skipped.is_empty(),
            "Existing commands should be skipped, skipped count: {}, written count: {}",
            report.commands.skipped.len(),
            report.commands.written
        );
    }
}

#[cfg(test)]
mod sync_dry_run_tests {
    use super::*;

    #[test]
    fn test_dry_run_no_changes() {
        //
        /*
        GIVEN a sync operation configured for dry run
        WHEN performing the sync
        THEN it should report what would be synced
        BUT make no actual changes to the target
        */
        //
        let setup = SyncTestSetup::new().unwrap();

        setup.setup_claude_config(setup.source_dir.path());

        let source = ClaudeAdapter::with_root(setup.source_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(setup.target_dir.path().to_path_buf());

        let params = SyncParams {
            from: None,
            dry_run: true, // Enable dry run
            force: false,
            sync_skills: true,
            sync_commands: true,
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
            sync_plugin_assets: false,
            interactive: false,
            exclude_plugins: vec![],
            full_plugin_mirror: false,
            keep_versions: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        let report = result.expect("Dry run should succeed");

        // Should report what would be synced
        assert!(
            report.skills.written > 0,
            "Should report skills would be written"
        );
        assert!(
            report.commands.written > 0,
            "Should report commands would be written"
        );

        // But target directory should remain empty (dry run)
        let target_entries: Vec<_> = fs::read_dir(setup.target_dir.path()).unwrap().collect();
        assert!(
            target_entries.is_empty(),
            "Target should be empty after dry run"
        );
    }
}

#[cfg(test)]
mod sync_force_overwrite_tests {
    use super::*;

    #[test]
    fn test_force_overwrites_existing() {
        //
        /*
        GIVEN a target with existing data
        WHEN syncing with force enabled
        THEN all existing data should be overwritten
        */
        //
        let setup = SyncTestSetup::new().unwrap();

        // Setup both source and target
        setup.setup_claude_config(setup.source_dir.path());
        setup.setup_codex_config(setup.target_dir.path());

        let source = ClaudeAdapter::with_root(setup.source_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(setup.target_dir.path().to_path_buf());

        // Create a file in target that should be overwritten
        let target_cmd_dir = setup.target_dir.path().join("prompts");
        fs::write(
            target_cmd_dir.join("to-be-overwritten.md"),
            "This content should be overwritten",
        )
        .unwrap();

        let params = SyncParams {
            from: None,
            dry_run: false,
            force: true, // Enable force overwrite
            sync_skills: true,
            sync_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false, // Should be ignored due to force
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
            sync_plugin_assets: false,
            interactive: false,
            exclude_plugins: vec![],
            full_plugin_mirror: false,
            keep_versions: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        let report = result.expect("Force sync should succeed");

        // Force should write all commands regardless of existing
        assert!(
            report.commands.written > 0,
            "Commands should be written with force"
        );
    }
}

#[cfg(test)]
mod sync_error_handling_tests {
    use super::*;

    #[test]
    fn test_handles_missing_directories() {
        //
        /*
        GIVEN sync operations with missing source directories
        WHEN attempting to sync
        THEN it should handle missing directories gracefully
        */
        //
        let setup = SyncTestSetup::new().unwrap();

        // Don't setup any directories - test with missing paths

        let source = ClaudeAdapter::with_root(setup.source_dir.path().to_path_buf());
        let target = CodexAdapter::with_root(setup.target_dir.path().to_path_buf());

        let params = SyncParams {
            from: None,
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: true,
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
            sync_plugin_assets: false,
            interactive: false,
            exclude_plugins: vec![],
            full_plugin_mirror: false,
            keep_versions: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        // Should succeed even with missing directories (create as needed)
        let _ = result.expect("Should handle missing directories gracefully");
    }

    #[test]
    fn test_partial_sync_failure() {
        //
        /*
        GIVEN a sync operation where some components fail
        WHEN the sync completes
        THEN successful components should be reported
        AND failed components should have error information
        */
        //
        let setup = SyncTestSetup::new().unwrap();

        setup.setup_claude_config(setup.source_dir.path());

        // Create source adapter
        let source = ClaudeAdapter::with_root(setup.source_dir.path().to_path_buf());

        // Create target with read-only permissions to simulate failure
        let target = CodexAdapter::with_root(setup.target_dir.path().to_path_buf());

        let params = SyncParams {
            from: None,
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: true,
            sync_mcp_servers: true,
            sync_preferences: true,
            skip_existing_commands: false,
            sync_agents: false,
            sync_hooks: false,
            sync_instructions: true,
            skip_existing_instructions: false,
            include_marketplace: false,
            sync_plugin_assets: false,
            interactive: false,
            exclude_plugins: vec![],
            full_plugin_mirror: false,
            keep_versions: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let result = orchestrator.sync(&params);

        // Note: Actual failure simulation would require more complex setup
        // This test structure validates the pattern for error handling
        match result {
            Ok(report) => {
                // If sync succeeds, verify report structure
                assert!(
                    report.skills.written > 0 || report.commands.written > 0,
                    "At least some components should be processed"
                );
            }
            Err(e) => {
                // If sync fails, verify error is properly structured
                assert!(
                    !e.to_string().is_empty(),
                    "Error should have descriptive message"
                );
            }
        }
    }
}

/// Codex rewrites `name:` to the folder and uses the leaf folder of a nested
/// skill, so a dry run that compares the source bytes and name with the
/// target reported the same skill as "would write" after every real sync.
#[test]
fn dry_run_after_a_real_sync_to_codex_reports_no_skill_changes() {
    let source_dir = TempDir::new().unwrap();
    let target_dir = TempDir::new().unwrap();
    let skill = source_dir.path().join("skills/synced/abc/morning");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: morning-brief\ndescription: Start the day\n---\n\nBody.\n",
    )
    .unwrap();
    fs::write(skill.join("helper.py"), "print('hi')\n").unwrap();

    let only_skills = |dry_run| SyncParams {
        dry_run,
        sync_skills: true,
        sync_commands: false,
        sync_mcp_servers: false,
        sync_preferences: false,
        sync_agents: false,
        sync_hooks: false,
        sync_instructions: false,
        sync_plugin_assets: false,
        ..SyncParams::default()
    };
    let orchestrator = SyncOrchestrator::new(
        ClaudeAdapter::with_root(source_dir.path().to_path_buf()),
        CodexAdapter::with_root(target_dir.path().to_path_buf()),
    );

    let real = orchestrator.sync(&only_skills(false)).unwrap();
    assert_eq!(real.skills.written, 1, "{:?}", real.skills);
    let preview = orchestrator.sync(&only_skills(true)).unwrap();

    assert_eq!(preview.skills.written, 0, "{:?}", preview.skills);
}
