//! Tests for the sync module's skip_existing_commands feature
//!
//! These tests validate the new functionality that prevents overwriting
//! existing commands during sync operations.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use tempfile::TempDir;

use sha2::{Digest, Sha256};

use skrills_sync::{
    adapters::traits::AgentAdapter,
    adapters::{ClaudeAdapter, CodexAdapter},
    common::Command,
    orchestrator::{SyncOrchestrator, SyncParams},
    report::SkipReason,
};

/// Test setup for sync operations
struct SyncTestContext {
    #[allow(dead_code)] // TempDir needs to be kept alive to prevent cleanup
    temp_dir: TempDir,
    source_dir: PathBuf,
    target_dir: PathBuf,
}

impl SyncTestContext {
    fn new() -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let source_dir = temp_dir.path().join("source");
        let target_dir = temp_dir.path().join("target");

        fs::create_dir_all(&source_dir)?;
        fs::create_dir_all(&target_dir)?;

        Ok(Self {
            temp_dir,
            source_dir,
            target_dir,
        })
    }

    /// Create a Claude adapter with specified commands
    fn create_claude_adapter_with_commands(&self, commands: Vec<Command>) -> ClaudeAdapter {
        let adapter = ClaudeAdapter::with_root(self.source_dir.clone());

        // Write commands to the filesystem (Claude uses "commands" directory)
        for cmd in commands {
            let cmd_path = adapter
                .config_root()
                .join("commands")
                .join(format!("{}.md", cmd.name));
            fs::create_dir_all(cmd_path.parent().unwrap()).unwrap();
            fs::write(cmd_path, cmd.content).unwrap();
        }

        adapter
    }

    /// Create a Codex adapter with specified commands
    fn create_codex_adapter_with_commands(&self, commands: Vec<Command>) -> CodexAdapter {
        let adapter = CodexAdapter::with_root(self.target_dir.clone());

        // Write commands to the filesystem (Codex uses "prompts" directory)
        for cmd in commands {
            let cmd_path = adapter
                .config_root()
                .join("prompts")
                .join(format!("{}.md", cmd.name));
            fs::create_dir_all(cmd_path.parent().unwrap()).unwrap();
            fs::write(cmd_path, cmd.content).unwrap();
        }

        adapter
    }

    /// Create sample command for testing
    fn create_sample_command(name: &str, content: &str) -> Command {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        Command {
            name: name.to_string(),
            content: content.as_bytes().to_vec(),
            source_path: PathBuf::from(format!("{}.md", name)),
            modified: SystemTime::now(),
            hash,
        }
    }
}

#[cfg(test)]
mod skip_existing_commands_tests {
    use super::*;

    #[tokio::test]
    async fn test_skip_existing_disabled_overwrites_all() {
        // GIVEN: a target with existing commands
        // WHEN: syncing with skip_existing_commands disabled
        // THEN: all commands should be overwritten
        let ctx = SyncTestContext::new().unwrap();

        // Setup source commands
        let source_commands = vec![
            SyncTestContext::create_sample_command("cmd1", "First command"),
            SyncTestContext::create_sample_command("cmd2", "Second command"),
            SyncTestContext::create_sample_command("cmd3", "Third command"),
        ];

        // Setup target with existing commands
        let target_commands = vec![
            SyncTestContext::create_sample_command("cmd1", "Old first command"),
            SyncTestContext::create_sample_command("cmd4", "Existing different command"),
        ];

        let source_adapter = ctx.create_claude_adapter_with_commands(source_commands);
        let target_adapter = ctx.create_codex_adapter_with_commands(target_commands);

        // Configure sync without skip_existing
        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: false, // Disabled - should overwrite
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source_adapter, target_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // Verify all source commands would be written
        assert_eq!(report.commands.written, 3, "Should write all 3 commands");
        assert_eq!(report.commands.skipped.len(), 0, "Should skip none");
    }

    #[tokio::test]
    async fn test_skip_existing_enabled_preserves_matching() {
        // GIVEN: a target with existing commands
        // WHEN: syncing with skip_existing_commands enabled
        // THEN: existing commands should be skipped and only new ones written
        let ctx = SyncTestContext::new().unwrap();

        // Setup source commands
        let source_commands = vec![
            SyncTestContext::create_sample_command("cmd1", "First command"),
            SyncTestContext::create_sample_command("cmd2", "Second command"),
            SyncTestContext::create_sample_command("cmd3", "Third command"),
        ];

        // Setup target with some overlapping commands
        let target_commands = vec![
            SyncTestContext::create_sample_command("cmd1", "Existing first command"),
            SyncTestContext::create_sample_command("cmd3", "Existing third command"),
        ];

        let source_adapter = ctx.create_claude_adapter_with_commands(source_commands);
        let target_adapter = ctx.create_codex_adapter_with_commands(target_commands);

        // Configure sync with skip_existing enabled
        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true, // Enabled - should skip existing
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source_adapter, target_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // Verify only new commands would be written
        assert_eq!(
            report.commands.written, 1,
            "Should write only 1 new command (cmd2)"
        );
        assert_eq!(
            report.commands.skipped.len(),
            2,
            "Should skip 2 existing commands"
        );

        // Verify correct commands were skipped
        let skipped_names: HashSet<String> = report
            .commands
            .skipped
            .iter()
            .filter_map(|reason| match reason {
                SkipReason::WouldOverwrite { item } => Some(item.clone()),
                _ => None,
            })
            .collect();

        assert!(skipped_names.contains("cmd1"), "Should skip cmd1");
        assert!(skipped_names.contains("cmd3"), "Should skip cmd3");
        assert!(!skipped_names.contains("cmd2"), "Should not skip cmd2");
    }

    #[tokio::test]
    async fn test_skip_existing_dry_run_mode() {
        //
        /*
        GIVEN sync with skip_existing_commands enabled in dry run mode
        WHEN performing the sync
        THEN it should report what would be skipped without making changes
        */
        //
        let ctx = SyncTestContext::new().unwrap();

        let source_commands = vec![
            SyncTestContext::create_sample_command("new-cmd", "New command"),
            SyncTestContext::create_sample_command("existing-cmd", "Updated command"),
        ];

        let target_commands = vec![SyncTestContext::create_sample_command(
            "existing-cmd",
            "Old command",
        )];

        let source_adapter = ctx.create_claude_adapter_with_commands(source_commands);
        let target_adapter = ctx.create_codex_adapter_with_commands(target_commands);

        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source_adapter, target_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // In dry run, should report what would happen
        assert_eq!(report.commands.written, 1, "Would write new-cmd");
        assert_eq!(report.commands.skipped.len(), 1, "Would skip existing-cmd");

        match &report.commands.skipped[0] {
            SkipReason::WouldOverwrite { item } => {
                assert_eq!(item, "existing-cmd");
            }
            _ => panic!("Expected WouldOverwrite skip reason"),
        }
    }

    #[tokio::test]
    async fn test_skip_existing_actual_sync() {
        //
        /*
        GIVEN sync with skip_existing_commands enabled in actual sync mode
        WHEN performing the sync
        THEN it should only write new commands and preserve existing ones
        */
        //
        let ctx = SyncTestContext::new().unwrap();

        let source_commands = vec![
            SyncTestContext::create_sample_command("write-me", "Should be written"),
            SyncTestContext::create_sample_command("skip-me", "Should be skipped"),
        ];

        let target_commands = vec![SyncTestContext::create_sample_command(
            "skip-me",
            "Already exists",
        )];

        let source_adapter = ctx.create_claude_adapter_with_commands(source_commands);
        let target_adapter = ctx.create_codex_adapter_with_commands(target_commands);

        let params = SyncParams {
            from: None,
            dry_run: false, // Actual sync
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source_adapter, target_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // Verify only new command was written
        assert_eq!(report.commands.written, 1, "Should write 1 new command");
        assert_eq!(
            report.commands.skipped.len(),
            1,
            "Should skip 1 existing command"
        );

        // Verify the target still has the original command
        let target_entries: Vec<_> = fs::read_dir(ctx.target_dir.join("prompts"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();

        assert!(
            target_entries.contains(&"skip-me.md".to_string()),
            "skip-me should still exist"
        );
    }

    #[tokio::test]
    async fn test_skip_existing_empty_target() {
        //
        /*
        GIVEN an empty target
        WHEN syncing with skip_existing_commands enabled
        THEN all commands should be written
        */
        //
        let ctx = SyncTestContext::new().unwrap();

        let source_commands = vec![
            SyncTestContext::create_sample_command("cmd1", "First"),
            SyncTestContext::create_sample_command("cmd2", "Second"),
        ];

        let target_commands = vec![]; // Empty target

        let source_adapter = ctx.create_claude_adapter_with_commands(source_commands);
        let target_adapter = ctx.create_codex_adapter_with_commands(target_commands);

        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source_adapter, target_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // All commands should be written when target is empty
        assert_eq!(report.commands.written, 2, "Should write all commands");
        assert_eq!(report.commands.skipped.len(), 0, "Should skip none");
    }

    #[tokio::test]
    async fn test_skip_existing_empty_source() {
        //
        /*
        GIVEN an empty source
        WHEN syncing with skip_existing_commands enabled
        THEN no commands should be written or skipped
        */
        //
        let ctx = SyncTestContext::new().unwrap();

        let source_commands = vec![]; // Empty source

        let target_commands = vec![SyncTestContext::create_sample_command(
            "existing",
            "Existing command",
        )];

        let source_adapter = ctx.create_claude_adapter_with_commands(source_commands);
        let target_adapter = ctx.create_codex_adapter_with_commands(target_commands);

        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source_adapter, target_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // No operations should occur
        assert_eq!(report.commands.written, 0, "Should write none");
        assert_eq!(report.commands.skipped.len(), 0, "Should skip none");
    }

    #[tokio::test]
    async fn test_skip_existing_reverse_direction() {
        //
        /*
        GIVEN skip_existing_commands enabled with reverse sync direction
        WHEN syncing from target to source
        THEN the same logic should apply in reverse
        */
        //
        let ctx = SyncTestContext::new().unwrap();

        // Source has fewer commands
        let claude_target_commands = vec![SyncTestContext::create_sample_command(
            "shared",
            "Source version",
        )];

        // Target has more commands including shared one
        let codex_source_commands = vec![
            SyncTestContext::create_sample_command("shared", "Target version"),
            SyncTestContext::create_sample_command("target-only", "Target only command"),
        ];

        let claude_adapter = ctx.create_claude_adapter_with_commands(claude_target_commands);
        let codex_adapter = ctx.create_codex_adapter_with_commands(codex_source_commands);

        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        // Sync from Codex (new source) to Claude (new target)
        let orchestrator = SyncOrchestrator::new(codex_adapter, claude_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // Should only write target-only command, skip shared
        assert_eq!(
            report.commands.written, 1,
            "Should write target-only command"
        );
        assert_eq!(
            report.commands.skipped.len(),
            1,
            "Should skip shared command"
        );
    }

    #[tokio::test]
    async fn test_skip_existing_with_force_flag() {
        //
        /*
        GIVEN skip_existing_commands enabled along with force flag
        WHEN syncing
        THEN force flag should take precedence and overwrite all
        */
        //
        let ctx = SyncTestContext::new().unwrap();

        let source_commands = vec![
            SyncTestContext::create_sample_command("cmd1", "Source version"),
            SyncTestContext::create_sample_command("cmd2", "New command"),
        ];

        let target_commands = vec![SyncTestContext::create_sample_command(
            "cmd1",
            "Target version",
        )];

        let source_adapter = ctx.create_claude_adapter_with_commands(source_commands);
        let target_adapter = ctx.create_codex_adapter_with_commands(target_commands);

        let params = SyncParams {
            from: None,
            dry_run: true,
            force: true, // Force takes precedence
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true, // Should be ignored due to force
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source_adapter, target_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // Force should overwrite all
        assert_eq!(
            report.commands.written, 2,
            "Force should write all commands"
        );
        assert_eq!(report.commands.skipped.len(), 0, "Force should skip none");
    }

    #[tokio::test]
    async fn test_skip_existing_case_sensitivity() {
        //
        /*
        GIVEN commands with different casing
        WHEN syncing with skip_existing_commands
        THEN command comparison should be case-sensitive
        */
        //
        let ctx = SyncTestContext::new().unwrap();

        let source_commands = vec![
            SyncTestContext::create_sample_command("Command", "Mixed case"),
            SyncTestContext::create_sample_command("command", "Lowercase"),
            SyncTestContext::create_sample_command("COMMAND", "Uppercase"),
        ];

        let target_commands = vec![SyncTestContext::create_sample_command(
            "command",
            "Existing lowercase",
        )];

        let source_adapter = ctx.create_claude_adapter_with_commands(source_commands);
        let target_adapter = ctx.create_codex_adapter_with_commands(target_commands);

        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: false,
            sync_commands: true,
            skip_existing_commands: true,
            sync_mcp_servers: false,
            sync_preferences: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source_adapter, target_adapter);
        let report = orchestrator.sync(&params).unwrap();

        // Only exact case match should be skipped
        assert_eq!(
            report.commands.written, 2,
            "Should write 2 commands (different case)"
        );
        assert_eq!(
            report.commands.skipped.len(),
            1,
            "Should skip 1 command (exact match)"
        );
    }
}
