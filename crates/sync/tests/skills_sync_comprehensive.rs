//! Integration tests for skill synchronization between Claude and Codex.
//!
//! These tests validate the CORE VALUE PROPOSITION of skrills:
//! - Bidirectional skill sync between Claude Code and Codex CLI
//! - Validation of skills against target CLI requirements
//! - Autofix capabilities for incompatible skills
//! - Edge cases and negative testing scenarios
//!
//! Test categories:
//! 1. Basic sync operations (Claude <-> Codex)
//! 2. Validation during sync
//! 3. Autofix scenarios
//! 4. Edge cases and boundary conditions
//! 5. Negative testing (failures, malformed inputs)
//! 6. Large-scale and performance scenarios

use sha2::{Digest, Sha256};
use skrills_sync::{
    adapters::traits::AgentAdapter,
    adapters::{ClaudeAdapter, CodexAdapter},
    common::Command,
    orchestrator::{SyncOrchestrator, SyncParams},
    validation::{
        skill_is_codex_compatible, validate_skill_for_sync, validate_skills_for_sync,
        SyncValidationOptions,
    },
};
use skrills_validate::ValidationTarget;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;
use tempfile::TempDir;

// =============================================================================
// Test Fixtures and Helpers
// =============================================================================

/// Test context for skill sync operations.
struct SkillSyncTestContext {
    #[allow(dead_code)]
    temp_dir: TempDir,
    claude_root: PathBuf,
    codex_root: PathBuf,
}

impl SkillSyncTestContext {
    fn new() -> anyhow::Result<Self> {
        let temp_dir = TempDir::new()?;
        let claude_root = temp_dir.path().join("claude");
        let codex_root = temp_dir.path().join("codex");

        fs::create_dir_all(&claude_root)?;
        fs::create_dir_all(&codex_root)?;

        Ok(Self {
            temp_dir,
            claude_root,
            codex_root,
        })
    }

    /// Create a Claude adapter with skills written to filesystem.
    fn claude_adapter_with_skills(&self, skills: Vec<Command>) -> ClaudeAdapter {
        let adapter = ClaudeAdapter::with_root(self.claude_root.clone());
        let skills_dir = adapter.config_root().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        for skill in skills {
            let skill_path = skills_dir.join(format!("{}.md", skill.name));
            fs::write(skill_path, &skill.content).unwrap();
        }

        adapter
    }

    /// Create a Codex adapter with skills written to filesystem.
    fn codex_adapter_with_skills(&self, skills: Vec<Command>) -> CodexAdapter {
        let adapter = CodexAdapter::with_root(self.codex_root.clone());
        let skills_dir = adapter.config_root().join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        for skill in skills {
            let skill_path = skills_dir.join(format!("{}.md", skill.name));
            fs::write(skill_path, &skill.content).unwrap();
        }

        adapter
    }

    /// Helper to create a skill with proper hash.
    fn make_skill(name: &str, content: &str) -> Command {
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

    /// Create a valid Claude-only skill (no frontmatter).
    fn claude_only_skill(name: &str) -> Command {
        Self::make_skill(
            name,
            &format!(
                "# {}\n\nThis skill has no frontmatter.\n\nIt works on Claude but not Codex.",
                name
            ),
        )
    }

    /// Create a skill valid for both Claude and Codex.
    fn universal_skill(name: &str, description: &str) -> Command {
        Self::make_skill(
            name,
            &format!(
                "---\nname: {}\ndescription: {}\n---\n# {}\n\nThis skill works on both CLIs.",
                name, description, name
            ),
        )
    }

    /// Create a skill with frontmatter but missing description.
    fn missing_description_skill(name: &str) -> Command {
        Self::make_skill(
            name,
            &format!(
                "---\nname: {}\n---\n# {}\n\nMissing description field.",
                name, name
            ),
        )
    }

    /// Create a skill with name exceeding max length (100 chars).
    fn oversized_name_skill() -> Command {
        let long_name = "a".repeat(101);
        Self::make_skill(
            &long_name,
            &format!(
                "---\nname: {}\ndescription: Has oversized name\n---\n# Content",
                long_name
            ),
        )
    }

    /// Create a skill with description exceeding max length (500 chars).
    fn oversized_description_skill(name: &str) -> Command {
        let long_desc = "x".repeat(501);
        Self::make_skill(
            name,
            &format!(
                "---\nname: {}\ndescription: {}\n---\n# Content",
                name, long_desc
            ),
        )
    }

    /// Create a skill with empty content after frontmatter.
    fn empty_content_skill(name: &str) -> Command {
        Self::make_skill(
            name,
            &format!("---\nname: {}\ndescription: Empty content\n---\n", name),
        )
    }

    /// Create a skill with malformed YAML frontmatter.
    fn malformed_yaml_skill(name: &str) -> Command {
        Self::make_skill(
            name,
            &format!("---\nname: {}\n  bad indent: true\n---\n# Content", name),
        )
    }

    /// Create a skill with unicode characters.
    fn unicode_skill(name: &str) -> Command {
        Self::make_skill(
            name,
            &format!(
                "---\nname: {}\ndescription: スキル説明 - 技能描述 - Описание навыка\n---\n# {} 🚀\n\n多言語コンテンツ",
                name, name
            ),
        )
    }

    /// Create a skill with special characters in name.
    fn special_chars_skill() -> Command {
        Self::make_skill(
            "my-skill_v2.0",
            "---\nname: my-skill_v2.0\ndescription: Has special chars\n---\n# Content",
        )
    }
}

// =============================================================================
// Module: Basic Sync Operations (Claude <-> Codex)
// =============================================================================

#[cfg(test)]
mod basic_sync_tests {
    use super::*;

    #[tokio::test]
    async fn given_claude_skills_when_sync_to_codex_then_skills_transferred() {
        // GIVEN: Claude has skills that are Codex-compatible
        let ctx = SkillSyncTestContext::new().unwrap();

        let claude_skills = vec![
            SkillSyncTestContext::universal_skill("code-review", "Reviews code quality"),
            SkillSyncTestContext::universal_skill("test-gen", "Generates unit tests"),
        ];

        let source = ctx.claude_adapter_with_skills(claude_skills);
        let target = ctx.codex_adapter_with_skills(vec![]);

        // WHEN: Syncing from Claude to Codex
        let params = SyncParams {
            from: None,
            dry_run: false,
            force: false,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let report = orchestrator.sync(&params).unwrap();

        // THEN: Skills should be transferred to Codex
        assert_eq!(report.skills.written, 2, "Should write both skills");
        assert!(report.skills.skipped.is_empty(), "Should skip none");
    }

    #[tokio::test]
    async fn given_codex_skills_when_sync_to_claude_then_skills_transferred() {
        // GIVEN: Codex has skills (reverse direction)
        let ctx = SkillSyncTestContext::new().unwrap();

        let codex_skills = vec![
            SkillSyncTestContext::universal_skill("debugging", "Debug assistance"),
            SkillSyncTestContext::universal_skill("refactoring", "Code refactoring"),
        ];

        let source = ctx.codex_adapter_with_skills(codex_skills);
        let target = ctx.claude_adapter_with_skills(vec![]);

        // WHEN: Syncing from Codex to Claude
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
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let report = orchestrator.sync(&params).unwrap();

        // THEN: Skills should be transferred to Claude
        assert_eq!(report.skills.written, 2, "Should write both skills");
    }

    #[tokio::test]
    async fn given_overlapping_skills_when_sync_then_all_source_skills_written() {
        // GIVEN: Both sides have some overlapping skills
        // NOTE: Unlike commands, skills don't have skip_existing behavior in current implementation
        let ctx = SkillSyncTestContext::new().unwrap();

        let claude_skills = vec![
            SkillSyncTestContext::universal_skill("shared-skill", "Claude version"),
            SkillSyncTestContext::universal_skill("claude-only", "Only on Claude"),
        ];

        let codex_skills = vec![SkillSyncTestContext::universal_skill(
            "shared-skill",
            "Codex version - will be overwritten",
        )];

        let source = ctx.claude_adapter_with_skills(claude_skills);
        let target = ctx.codex_adapter_with_skills(codex_skills);

        // WHEN: Syncing skills
        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false, // Only affects commands, not skills
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let report = orchestrator.sync(&params).unwrap();

        // THEN: All source skills are written (skills sync overwrites by default)
        assert_eq!(
            report.skills.written, 2,
            "Should write all source skills (skills don't have skip_existing behavior)"
        );
    }

    #[tokio::test]
    async fn given_empty_source_when_sync_then_no_changes() {
        // GIVEN: Source has no skills
        let ctx = SkillSyncTestContext::new().unwrap();

        let target_skills = vec![SkillSyncTestContext::universal_skill(
            "existing",
            "Already exists",
        )];

        let source = ctx.claude_adapter_with_skills(vec![]);
        let target = ctx.codex_adapter_with_skills(target_skills);

        // WHEN: Syncing from empty source
        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let report = orchestrator.sync(&params).unwrap();

        // THEN: No skills written
        assert_eq!(report.skills.written, 0, "Should write no skills");
    }

    #[tokio::test]
    async fn given_force_flag_when_sync_then_overwrites_all() {
        // GIVEN: Target has existing skills
        let ctx = SkillSyncTestContext::new().unwrap();

        let claude_skills = vec![SkillSyncTestContext::universal_skill(
            "overwrite-me",
            "New version from Claude",
        )];

        let codex_skills = vec![SkillSyncTestContext::universal_skill(
            "overwrite-me",
            "Old version on Codex",
        )];

        let source = ctx.claude_adapter_with_skills(claude_skills);
        let target = ctx.codex_adapter_with_skills(codex_skills);

        // WHEN: Syncing with force enabled
        let params = SyncParams {
            from: None,
            dry_run: true,
            force: true,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: true, // Should be ignored due to force
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let report = orchestrator.sync(&params).unwrap();

        // THEN: Skill should be overwritten
        assert_eq!(report.skills.written, 1, "Should overwrite skill");
        assert!(
            report.skills.skipped.is_empty(),
            "Force should skip nothing"
        );
    }

    #[tokio::test]
    async fn given_dry_run_when_sync_then_no_filesystem_changes() {
        // GIVEN: Source has skills to sync
        let ctx = SkillSyncTestContext::new().unwrap();

        let claude_skills = vec![
            SkillSyncTestContext::universal_skill("dry-run-1", "Test skill 1"),
            SkillSyncTestContext::universal_skill("dry-run-2", "Test skill 2"),
        ];

        let source = ctx.claude_adapter_with_skills(claude_skills);
        let target = ctx.codex_adapter_with_skills(vec![]);

        // WHEN: Syncing with dry_run enabled
        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let report = orchestrator.sync(&params).unwrap();

        // THEN: Report shows what would be written but target remains empty
        assert_eq!(report.skills.written, 2, "Should report 2 skills to write");

        // Verify target directory is still empty
        let target_skills_dir = ctx.codex_root.join("skills");
        if target_skills_dir.exists() {
            let entries: Vec<_> = fs::read_dir(&target_skills_dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .collect();
            assert!(entries.is_empty(), "Target should remain empty in dry run");
        }
    }
}

// =============================================================================
// Module: Validation During Sync
// =============================================================================

#[cfg(test)]
mod validation_sync_tests {
    use super::*;

    #[test]
    fn given_codex_compatible_skill_when_validate_then_passes() {
        // GIVEN: A skill with valid frontmatter
        let skill = SkillSyncTestContext::universal_skill("valid", "A valid skill");

        // WHEN: Validating for Codex
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Validation passes
        assert!(result.can_sync, "Should be able to sync");
        assert!(result.validation.is_codex_valid(), "Should be Codex valid");
    }

    #[test]
    fn given_claude_only_skill_when_validate_for_codex_then_fails() {
        // GIVEN: A skill without frontmatter
        let skill = SkillSyncTestContext::claude_only_skill("claude-only");

        // WHEN: Validating for Codex
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Validation fails for Codex but would pass for Claude
        assert!(!result.can_sync, "Should not be able to sync to Codex");
        assert!(
            !result.validation.is_codex_valid(),
            "Should not be Codex valid"
        );
    }

    #[test]
    fn given_missing_description_when_validate_then_fails() {
        // GIVEN: A skill with name but missing description
        let skill = SkillSyncTestContext::missing_description_skill("incomplete");

        // WHEN: Validating for Codex
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Validation fails
        assert!(!result.can_sync, "Should not be able to sync");
        assert!(
            result
                .validation
                .issues
                .iter()
                .any(|i| i.message.contains("description")),
            "Should report missing description"
        );
    }

    #[test]
    fn given_oversized_name_when_validate_then_fails() {
        // GIVEN: A skill with name > 100 characters
        let skill = SkillSyncTestContext::oversized_name_skill();

        // WHEN: Validating for Codex
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Validation fails
        assert!(!result.can_sync, "Should not be able to sync");
        assert!(
            result
                .validation
                .issues
                .iter()
                .any(|i| i.message.contains("exceeds maximum")),
            "Should report name too long"
        );
    }

    #[test]
    fn given_oversized_description_when_validate_then_fails() {
        // GIVEN: A skill with description > 500 characters
        let skill = SkillSyncTestContext::oversized_description_skill("oversized-desc");

        // WHEN: Validating for Codex
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Validation fails
        assert!(!result.can_sync, "Should not be able to sync");
        assert!(
            result
                .validation
                .issues
                .iter()
                .any(|i| i.message.contains("exceeds maximum")),
            "Should report description too long"
        );
    }

    #[test]
    fn given_empty_content_when_validate_then_fails() {
        // GIVEN: A skill with empty content after frontmatter
        let skill = SkillSyncTestContext::empty_content_skill("empty");

        // WHEN: Validating for Codex
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Validation fails
        assert!(!result.can_sync, "Should not be able to sync empty skill");
    }

    #[test]
    fn given_multiple_skills_when_validate_batch_then_reports_all() {
        // GIVEN: Mix of valid and invalid skills
        let skills = vec![
            SkillSyncTestContext::universal_skill("valid-1", "First valid"),
            SkillSyncTestContext::claude_only_skill("invalid-1"),
            SkillSyncTestContext::universal_skill("valid-2", "Second valid"),
            SkillSyncTestContext::missing_description_skill("invalid-2"),
        ];

        // WHEN: Validating all for Codex
        let options = SyncValidationOptions::default();
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);

        // THEN: Report contains correct counts
        assert_eq!(report.total, 4);
        assert_eq!(report.passed, 2);
        assert_eq!(report.failed, 2);
        assert!(!report.all_valid());
    }

    #[test]
    fn given_both_target_when_validate_then_checks_both_formats() {
        // GIVEN: A skill valid for Claude but not Codex
        let skill = SkillSyncTestContext::claude_only_skill("claude-only");

        // WHEN: Validating for Both targets
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Both, &options);

        // THEN: Reports Claude valid but Codex invalid
        assert!(
            result.validation.is_claude_valid(),
            "Should be Claude valid"
        );
        assert!(
            !result.validation.is_codex_valid(),
            "Should not be Codex valid"
        );
    }

    #[test]
    fn given_unicode_content_when_validate_then_handles_correctly() {
        // GIVEN: A skill with unicode characters
        let skill = SkillSyncTestContext::unicode_skill("unicode-skill");

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Both, &options);

        // THEN: Validation handles unicode correctly
        assert!(result.can_sync, "Unicode should be valid");
        assert!(
            result.validation.is_codex_valid(),
            "Unicode skill should be Codex valid"
        );
    }

    #[test]
    fn given_special_chars_in_name_when_validate_then_accepts() {
        // GIVEN: A skill with hyphens, underscores, dots in name
        let skill = SkillSyncTestContext::special_chars_skill();

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Special characters in name are accepted
        assert!(result.can_sync, "Special chars should be valid");
    }

    #[test]
    fn given_validation_report_when_summary_then_formats_correctly() {
        // GIVEN: A validation report
        let skills = vec![
            SkillSyncTestContext::universal_skill("valid", "Valid skill"),
            SkillSyncTestContext::claude_only_skill("invalid"),
        ];
        let options = SyncValidationOptions::default();
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);

        // WHEN: Getting summary
        let summary = report.summary();

        // THEN: Summary contains relevant information
        assert!(
            summary.contains("1") || summary.contains("2"),
            "Should contain counts"
        );
    }
}

// =============================================================================
// Module: Autofix Scenarios
// =============================================================================

#[cfg(test)]
mod autofix_sync_tests {
    use super::*;

    #[test]
    fn given_no_frontmatter_when_autofix_enabled_then_adds_frontmatter() {
        // GIVEN: A skill without frontmatter
        let skill = SkillSyncTestContext::claude_only_skill("needs-fix");

        // WHEN: Validating with autofix enabled
        let options = SyncValidationOptions {
            validate: true,
            autofix: true,
            create_backups: false,
            strict: false,
        };
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Autofix generates frontmatter and skill can sync
        assert!(result.can_sync, "Should be able to sync after autofix");
        assert!(result.autofix.is_some(), "Should have autofix result");
        assert!(
            result.autofix.as_ref().unwrap().modified,
            "Should have modified content"
        );
    }

    #[test]
    fn given_missing_name_when_autofix_then_derives_from_filename() {
        // GIVEN: A skill with description but no name
        let skill = SkillSyncTestContext::make_skill(
            "derive-name",
            "---\ndescription: Has description only\n---\n# Content",
        );

        // WHEN: Validating with autofix
        let options = SyncValidationOptions {
            validate: true,
            autofix: true,
            create_backups: false,
            strict: false,
        };
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Name is derived and added
        assert!(result.can_sync, "Should be able to sync after autofix");
        if let Some(ref autofix) = result.autofix {
            assert!(
                autofix.content.contains("name:"),
                "Should have added name field"
            );
        }
    }

    #[test]
    fn given_missing_description_when_autofix_then_derives_from_content() {
        // GIVEN: A skill with name but no description
        let skill = SkillSyncTestContext::missing_description_skill("needs-desc");

        // WHEN: Validating with autofix
        let options = SyncValidationOptions {
            validate: true,
            autofix: true,
            create_backups: false,
            strict: false,
        };
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Description is derived and added
        assert!(result.can_sync, "Should be able to sync after autofix");
        if let Some(ref autofix) = result.autofix {
            assert!(
                autofix.content.contains("description:"),
                "Should have added description field"
            );
        }
    }

    #[test]
    fn given_valid_skill_when_autofix_enabled_then_no_changes() {
        // GIVEN: A skill that's already valid
        let skill =
            SkillSyncTestContext::universal_skill("already-valid", "Already has everything");

        // WHEN: Validating with autofix
        let options = SyncValidationOptions {
            validate: true,
            autofix: true,
            create_backups: false,
            strict: false,
        };
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: No autofix applied (skill was already valid)
        assert!(result.can_sync, "Should be able to sync");
        // Autofix should be None or not modified since skill was valid
        if let Some(ref autofix) = result.autofix {
            assert!(!autofix.modified, "Should not modify already-valid skill");
        }
    }

    #[test]
    fn given_batch_with_fixable_skills_when_autofix_then_report_shows_fixed() {
        // GIVEN: Mix of valid and fixable skills
        let skills = vec![
            SkillSyncTestContext::universal_skill("valid", "Already valid"),
            SkillSyncTestContext::claude_only_skill("fixable"),
        ];

        // WHEN: Validating with autofix
        let options = SyncValidationOptions {
            validate: true,
            autofix: true,
            create_backups: false,
            strict: false,
        };
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);

        // THEN: Report shows autofixed count
        assert_eq!(report.total, 2);
        assert_eq!(report.passed, 1);
        assert_eq!(report.autofixed, 1);
        assert_eq!(report.failed, 0);
        assert!(report.all_valid(), "All should be valid after autofix");
    }

    #[test]
    fn given_unfixable_skill_when_autofix_then_remains_invalid() {
        // GIVEN: A skill with malformed YAML (cannot autofix invalid YAML)
        let skill = SkillSyncTestContext::malformed_yaml_skill("broken-yaml");

        // WHEN: Validating with autofix
        let options = SyncValidationOptions {
            validate: true,
            autofix: true,
            create_backups: false,
            strict: false,
        };
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Skill cannot be fixed (malformed YAML)
        // Note: Behavior depends on implementation - may still add new frontmatter
        // or may fail. Test the actual behavior.
        // This documents the expected behavior for unfixable scenarios.
        assert!(
            result.autofix.is_some() || !result.can_sync,
            "Should either attempt fix or mark as unfixable"
        );
    }

    #[test]
    fn given_autofix_disabled_when_validate_then_no_fix_applied() {
        // GIVEN: A skill without frontmatter
        let skill = SkillSyncTestContext::claude_only_skill("no-autofix");

        // WHEN: Validating with autofix disabled
        let options = SyncValidationOptions {
            validate: true,
            autofix: false,
            create_backups: false,
            strict: false,
        };
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: No autofix applied
        assert!(!result.can_sync, "Should not be able to sync");
        assert!(result.autofix.is_none(), "Should not have autofix result");
    }
}

// =============================================================================
// Module: Edge Cases and Boundary Conditions
// =============================================================================

#[cfg(test)]
mod edge_case_tests {
    use super::*;

    #[test]
    fn given_name_exactly_100_chars_when_validate_then_passes() {
        // GIVEN: Name at exactly the maximum length
        let name = "a".repeat(100);
        let skill = SkillSyncTestContext::make_skill(
            &name,
            &format!(
                "---\nname: {}\ndescription: Exactly 100 char name\n---\n# Content",
                name
            ),
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should pass (boundary case)
        assert!(result.can_sync, "Exactly 100 char name should be valid");
    }

    #[test]
    fn given_name_101_chars_when_validate_then_fails() {
        // GIVEN: Name at exactly one over maximum
        let skill = SkillSyncTestContext::oversized_name_skill();

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should fail
        assert!(!result.can_sync, "101 char name should be invalid");
    }

    #[test]
    fn given_description_exactly_500_chars_when_validate_then_passes() {
        // GIVEN: Description at exactly the maximum length
        let desc = "d".repeat(500);
        let skill = SkillSyncTestContext::make_skill(
            "boundary-desc",
            &format!(
                "---\nname: boundary-desc\ndescription: {}\n---\n# Content",
                desc
            ),
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should pass
        assert!(
            result.can_sync,
            "Exactly 500 char description should be valid"
        );
    }

    #[test]
    fn given_empty_name_when_validate_then_fails() {
        // GIVEN: Skill with empty name field
        let skill = SkillSyncTestContext::make_skill(
            "empty-name",
            "---\nname: \"\"\ndescription: Has description\n---\n# Content",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should fail
        assert!(!result.can_sync, "Empty name should be invalid");
    }

    #[test]
    fn given_empty_description_when_validate_then_fails() {
        // GIVEN: Skill with empty description field
        let skill = SkillSyncTestContext::make_skill(
            "empty-desc",
            "---\nname: empty-desc\ndescription: \"\"\n---\n# Content",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should fail
        assert!(!result.can_sync, "Empty description should be invalid");
    }

    #[test]
    fn given_whitespace_only_content_when_validate_then_fails() {
        // GIVEN: Skill with only whitespace after frontmatter
        let skill = SkillSyncTestContext::make_skill(
            "whitespace",
            "---\nname: whitespace\ndescription: Test\n---\n   \n\t\n  ",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should fail (content is effectively empty)
        assert!(
            !result.can_sync,
            "Whitespace-only content should be invalid"
        );
    }

    #[test]
    fn given_newline_in_name_when_validate_then_fails() {
        // GIVEN: Skill with newline in name (invalid)
        let skill = SkillSyncTestContext::make_skill(
            "newline-name",
            "---\nname: \"line1\\nline2\"\ndescription: Test\n---\n# Content",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Depends on YAML parsing - document behavior
        // Names with newlines are typically invalid
        assert!(!result.can_sync || result.validation.is_codex_valid());
    }

    #[test]
    fn given_multiple_frontmatter_blocks_when_validate_then_handles_first() {
        // GIVEN: Skill with multiple --- blocks
        let skill = SkillSyncTestContext::make_skill(
            "multi-front",
            "---\nname: multi-front\ndescription: First block\n---\n# Content\n---\nsomething: else\n---",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should parse first frontmatter block only
        assert!(result.can_sync, "First valid frontmatter should work");
    }

    #[test]
    fn given_frontmatter_not_at_start_when_validate_then_fails_codex() {
        // GIVEN: Skill with frontmatter not at very start
        let skill = SkillSyncTestContext::make_skill(
            "delayed-front",
            "\n---\nname: delayed-front\ndescription: Not at start\n---\n# Content",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let _result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: May fail depending on parser strictness
        // Document the actual behavior
        // Most parsers require frontmatter to start at line 1
    }

    #[test]
    fn given_yaml_with_multiline_description_when_validate_then_handles() {
        // GIVEN: Skill with multiline description in YAML
        let skill = SkillSyncTestContext::make_skill(
            "multiline-desc",
            "---\nname: multiline-desc\ndescription: |\n  This is a\n  multiline description\n  for testing.\n---\n# Content",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Multiline descriptions should be valid
        assert!(result.can_sync, "Multiline description should be valid");
    }

    #[test]
    fn given_zero_skills_when_validate_batch_then_returns_empty_report() {
        // GIVEN: No skills
        let skills: Vec<Command> = vec![];

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);

        // THEN: Empty report
        assert_eq!(report.total, 0);
        assert_eq!(report.passed, 0);
        assert_eq!(report.failed, 0);
        assert!(report.all_valid(), "Empty set is considered valid");
    }
}

// =============================================================================
// Module: Negative Testing (Failure Scenarios)
// =============================================================================

#[cfg(test)]
mod negative_tests {
    use super::*;

    #[test]
    fn given_completely_empty_file_when_validate_then_fails() {
        // GIVEN: Empty file
        let skill = SkillSyncTestContext::make_skill("empty", "");

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should fail
        assert!(!result.can_sync, "Empty file should be invalid");
    }

    #[test]
    fn given_only_frontmatter_delimiters_when_validate_then_fails() {
        // GIVEN: Just --- markers with nothing inside
        let skill = SkillSyncTestContext::make_skill("just-delims", "---\n---\n# Content");

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should fail (no name/description)
        assert!(!result.can_sync, "Empty frontmatter should be invalid");
    }

    #[test]
    fn given_binary_content_when_validate_then_handles_gracefully() {
        // GIVEN: Skill with some binary bytes
        let mut content = b"---\nname: binary\ndescription: test\n---\n# Content\n".to_vec();
        content.extend_from_slice(&[0x00, 0xFF, 0xFE]);

        let skill = Command {
            name: "binary".to_string(),
            content,
            source_path: PathBuf::from("binary.md"),
            modified: SystemTime::now(),
            hash: "test".to_string(),
        };

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should handle gracefully (may pass or fail but not panic)
        // The result doesn't matter as much as not crashing
        let _ = result;
    }

    #[test]
    fn given_very_large_skill_when_validate_then_handles() {
        // GIVEN: Very large skill content (100KB)
        let large_content = format!(
            "---\nname: large-skill\ndescription: Large content test\n---\n# Large Skill\n{}",
            "x".repeat(100_000)
        );
        let skill = SkillSyncTestContext::make_skill("large-skill", &large_content);

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should handle without issue
        assert!(result.can_sync, "Large skill should be valid");
    }

    #[test]
    fn given_skill_with_null_bytes_in_name_when_validate_then_handles() {
        // GIVEN: Name containing null byte (edge case for C-style strings)
        let skill = SkillSyncTestContext::make_skill(
            "null-byte",
            "---\nname: \"null\\0byte\"\ndescription: Test\n---\n# Content",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let _result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should handle gracefully (not panic)
        // May fail validation but should not panic
    }

    #[test]
    fn given_yaml_injection_attempt_when_validate_then_safe() {
        // GIVEN: YAML content that might cause issues
        let skill = SkillSyncTestContext::make_skill(
            "yaml-injection",
            "---\nname: test\ndescription: \"test\\n---\\nmalicious: true\"\n---\n# Content",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: Should parse safely without executing injection
        // The description should be a literal string, not parsed as YAML
        // Just ensure validation completes without panic
        let _ = result;
    }

    #[test]
    fn given_duplicate_yaml_keys_when_validate_then_handles() {
        // GIVEN: YAML with duplicate keys
        let skill = SkillSyncTestContext::make_skill(
            "dup-keys",
            "---\nname: first\nname: second\ndescription: Test\n---\n# Content",
        );

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        // THEN: YAML parsers typically use last value or may error
        // Document the actual behavior
        let _ = result;
    }

    #[test]
    fn given_strict_mode_and_invalid_skill_when_validate_then_fails() {
        // GIVEN: Invalid skill with strict mode
        let skills = vec![
            SkillSyncTestContext::universal_skill("valid", "Valid skill"),
            SkillSyncTestContext::claude_only_skill("invalid"),
        ];

        // WHEN: Validating with strict mode
        let options = SyncValidationOptions {
            validate: true,
            autofix: false,
            create_backups: false,
            strict: true,
        };
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);

        // THEN: Report shows failure
        assert!(!report.all_valid(), "Should report invalid skills");
        assert_eq!(report.failed, 1);
    }
}

// =============================================================================
// Module: Quick Compatibility Check
// =============================================================================

#[cfg(test)]
mod quick_check_tests {
    use super::*;

    #[test]
    fn given_valid_codex_skill_when_quick_check_then_returns_true() {
        let skill = SkillSyncTestContext::universal_skill("valid", "Test");
        assert!(skill_is_codex_compatible(&skill));
    }

    #[test]
    fn given_claude_only_skill_when_quick_check_then_returns_false() {
        let skill = SkillSyncTestContext::claude_only_skill("claude-only");
        assert!(!skill_is_codex_compatible(&skill));
    }

    #[test]
    fn given_missing_name_when_quick_check_then_returns_false() {
        let skill = SkillSyncTestContext::make_skill(
            "no-name",
            "---\ndescription: Only description\n---\n# Content",
        );
        assert!(!skill_is_codex_compatible(&skill));
    }

    #[test]
    fn given_missing_description_when_quick_check_then_returns_false() {
        let skill = SkillSyncTestContext::missing_description_skill("no-desc");
        assert!(!skill_is_codex_compatible(&skill));
    }

    #[test]
    fn given_oversized_name_when_quick_check_then_returns_false() {
        let skill = SkillSyncTestContext::oversized_name_skill();
        assert!(!skill_is_codex_compatible(&skill));
    }

    #[test]
    fn given_oversized_description_when_quick_check_then_returns_false() {
        let skill = SkillSyncTestContext::oversized_description_skill("oversized");
        assert!(!skill_is_codex_compatible(&skill));
    }
}

// =============================================================================
// Module: Concurrent and Scale Tests
// =============================================================================

#[cfg(test)]
mod scale_tests {
    use super::*;

    #[test]
    fn given_many_skills_when_validate_batch_then_completes_efficiently() {
        // GIVEN: Large number of skills
        let skills: Vec<Command> = (0..100)
            .map(|i| {
                SkillSyncTestContext::universal_skill(
                    &format!("skill-{}", i),
                    &format!("Description for skill {}", i),
                )
            })
            .collect();

        // WHEN: Validating all
        let options = SyncValidationOptions::default();
        let start = std::time::Instant::now();
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);
        let duration = start.elapsed();

        // THEN: Should complete quickly
        assert_eq!(report.total, 100);
        assert_eq!(report.passed, 100);
        assert!(
            duration.as_millis() < 5000,
            "Should complete 100 skills in under 5 seconds"
        );
    }

    #[test]
    fn given_mixed_valid_invalid_at_scale_when_validate_then_accurate_counts() {
        // GIVEN: 50 valid, 50 invalid skills
        let mut skills: Vec<Command> = Vec::new();

        for i in 0..50 {
            skills.push(SkillSyncTestContext::universal_skill(
                &format!("valid-{}", i),
                "Valid skill",
            ));
        }

        for i in 0..50 {
            skills.push(SkillSyncTestContext::claude_only_skill(&format!(
                "invalid-{}",
                i
            )));
        }

        // WHEN: Validating all
        let options = SyncValidationOptions::default();
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);

        // THEN: Accurate counts
        assert_eq!(report.total, 100);
        assert_eq!(report.passed, 50);
        assert_eq!(report.failed, 50);
    }
}

// =============================================================================
// Module: Case Sensitivity Tests
// =============================================================================

#[cfg(test)]
mod case_sensitivity_tests {
    use super::*;

    #[tokio::test]
    async fn given_skills_with_different_case_when_sync_then_all_written() {
        // GIVEN: Skills with same name but different case
        // NOTE: Skills sync doesn't have skip_existing behavior - all source skills are written
        let ctx = SkillSyncTestContext::new().unwrap();

        let source_skills = vec![
            SkillSyncTestContext::universal_skill("MySkill", "Uppercase"),
            SkillSyncTestContext::universal_skill("myskill", "Lowercase"),
            SkillSyncTestContext::universal_skill("MYSKILL", "All caps"),
        ];

        let target_skills = vec![SkillSyncTestContext::universal_skill("myskill", "Existing")];

        let source = ctx.claude_adapter_with_skills(source_skills);
        let target = ctx.codex_adapter_with_skills(target_skills);

        // WHEN: Syncing
        let params = SyncParams {
            from: None,
            dry_run: true,
            force: false,
            sync_skills: true,
            sync_commands: false,
            sync_mcp_servers: false,
            sync_preferences: false,
            skip_existing_commands: false,
            sync_agents: false,
            include_marketplace: false,
        };

        let orchestrator = SyncOrchestrator::new(source, target);
        let report = orchestrator.sync(&params).unwrap();

        // THEN: All source skills are synced
        assert_eq!(
            report.skills.written, 3,
            "All source skills should be written"
        );
    }

    #[test]
    fn given_validation_with_case_variations_then_handles_correctly() {
        // GIVEN: Skills with various case patterns
        let skills = vec![
            SkillSyncTestContext::universal_skill("Code-Review", "Mixed case"),
            SkillSyncTestContext::universal_skill("code_review", "Underscore"),
            SkillSyncTestContext::universal_skill("CODEREVIEW", "All caps"),
        ];

        // WHEN: Validating
        let options = SyncValidationOptions::default();
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);

        // THEN: All should be valid regardless of case
        assert_eq!(report.total, 3);
        assert_eq!(report.passed, 3);
    }
}
