//! Validation integration for skill sync.
//!
//! Provides validation and autofix capabilities during sync operations
//! to ensure skills are compatible with their target CLI.

use crate::common::Command;
use skrills_validate::{
    autofix_frontmatter, is_codex_compatible, is_copilot_compatible, validate_skill,
    AutofixOptions, AutofixResult, ValidationResult, ValidationTarget,
};

/// Options for validation during sync.
#[derive(Debug, Clone, Default)]
pub struct SyncValidationOptions {
    /// Whether to validate skills before syncing.
    pub validate: bool,
    /// Whether to auto-fix skills that fail validation.
    pub autofix: bool,
    /// Create backups before autofix.
    pub create_backups: bool,
    /// Stop sync if validation fails.
    pub strict: bool,
}

/// Result of validating a skill during sync.
#[derive(Debug, Clone)]
pub struct SkillValidationResult {
    /// Skill name.
    pub name: String,
    /// Original path.
    pub source_path: std::path::PathBuf,
    /// Validation result.
    pub validation: ValidationResult,
    /// Autofix result (if applied).
    pub autofix: Option<AutofixResult>,
    /// Whether the skill can be synced to the target.
    pub can_sync: bool,
}

/// Report of all skill validations.
#[derive(Debug, Default)]
pub struct ValidationReport {
    /// Individual skill results.
    pub skills: Vec<SkillValidationResult>,
    /// Skills that passed validation.
    pub passed: usize,
    /// Skills that failed but were auto-fixed.
    pub autofixed: usize,
    /// Skills that failed validation.
    pub failed: usize,
    /// Total skills processed.
    pub total: usize,
}

impl ValidationReport {
    /// Check if all skills passed or were fixed.
    pub fn all_valid(&self) -> bool {
        self.failed == 0
    }

    /// Get a summary message.
    pub fn summary(&self) -> String {
        if self.all_valid() {
            if self.autofixed > 0 {
                format!(
                    "{} skills validated ({} auto-fixed)",
                    self.total, self.autofixed
                )
            } else {
                format!("{} skills validated successfully", self.total)
            }
        } else {
            format!(
                "{}/{} skills passed validation ({} failed)",
                self.passed + self.autofixed,
                self.total,
                self.failed
            )
        }
    }
}

/// Validate a single skill for sync to a target.
pub fn validate_skill_for_sync(
    skill: &Command,
    target: ValidationTarget,
    options: &SyncValidationOptions,
) -> SkillValidationResult {
    let content = String::from_utf8_lossy(&skill.content);
    let validation = validate_skill(&skill.source_path, &content, target);

    let is_valid = match target {
        ValidationTarget::Claude => validation.claude_valid,
        ValidationTarget::Codex => validation.codex_valid,
        ValidationTarget::Copilot => validation.copilot_valid,
        ValidationTarget::Both => validation.claude_valid && validation.codex_valid,
        ValidationTarget::All => {
            validation.claude_valid && validation.codex_valid && validation.copilot_valid
        }
    };

    let mut autofix_result = None;
    let mut can_sync = is_valid;

    // If validation failed and autofix is enabled, try to fix
    if !is_valid && options.autofix {
        let autofix_opts = AutofixOptions {
            create_backup: options.create_backups,
            write_changes: false, // We'll handle writing separately
            suggested_name: Some(skill.name.clone()),
            suggested_description: None,
        };

        if let Ok(result) = autofix_frontmatter(&skill.source_path, &content, &autofix_opts) {
            if result.modified {
                can_sync = true;
            }
            autofix_result = Some(result);
        }
    }

    SkillValidationResult {
        name: skill.name.clone(),
        source_path: skill.source_path.clone(),
        validation,
        autofix: autofix_result,
        can_sync,
    }
}

/// Validate all skills for sync to a target.
pub fn validate_skills_for_sync(
    skills: &[Command],
    target: ValidationTarget,
    options: &SyncValidationOptions,
) -> ValidationReport {
    let mut report = ValidationReport {
        total: skills.len(),
        ..Default::default()
    };

    for skill in skills {
        let result = validate_skill_for_sync(skill, target, options);

        if result.validation.claude_valid
            && result.validation.codex_valid
            && result.validation.copilot_valid
        {
            report.passed += 1;
        } else if result.autofix.as_ref().map(|a| a.modified).unwrap_or(false) {
            report.autofixed += 1;
        } else if !result.can_sync {
            report.failed += 1;
        } else {
            report.passed += 1;
        }

        report.skills.push(result);
    }

    report
}

/// Quick check if a skill is compatible with Codex.
pub fn skill_is_codex_compatible(skill: &Command) -> bool {
    let content = String::from_utf8_lossy(&skill.content);
    is_codex_compatible(&content)
}

/// Quick check if a skill is compatible with Copilot.
pub fn skill_is_copilot_compatible(skill: &Command) -> bool {
    let content = String::from_utf8_lossy(&skill.content);
    is_copilot_compatible(&content)
}

/// Apply autofix to a skill and return the modified content.
pub fn apply_autofix_to_skill(skill: &Command) -> Result<Vec<u8>, String> {
    let content = String::from_utf8_lossy(&skill.content);

    let options = AutofixOptions {
        create_backup: false,
        write_changes: false,
        suggested_name: Some(skill.name.clone()),
        suggested_description: None,
    };

    let result = autofix_frontmatter(&skill.source_path, &content, &options)?;

    if result.modified {
        Ok(result.content.into_bytes())
    } else {
        Ok(skill.content.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn make_skill(name: &str, content: &str) -> Command {
        Command {
            name: name.to_string(),
            content: content.as_bytes().to_vec(),
            source_path: format!("{}.md", name).into(),
            modified: SystemTime::now(),
            hash: "test".to_string(),
        }
    }

    #[test]
    fn test_validate_valid_skill() {
        let skill = make_skill(
            "test",
            "---\nname: test\ndescription: A test skill\n---\n# Content",
        );
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        assert!(result.can_sync);
        assert!(result.validation.codex_valid);
    }

    #[test]
    fn test_validate_invalid_skill_without_autofix() {
        let skill = make_skill("test", "# No frontmatter");
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        assert!(!result.can_sync);
        assert!(!result.validation.codex_valid);
    }

    #[test]
    fn test_validate_invalid_skill_with_autofix() {
        let skill = make_skill("test", "# No frontmatter\nBut has content.");
        let options = SyncValidationOptions {
            autofix: true,
            ..Default::default()
        };
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        assert!(result.can_sync);
        assert!(result.autofix.is_some());
        assert!(result.autofix.unwrap().modified);
    }

    #[test]
    fn test_skill_is_codex_compatible() {
        let valid = make_skill(
            "valid",
            "---\nname: valid\ndescription: Test\n---\n# Content",
        );
        let invalid = make_skill("invalid", "# No frontmatter");

        assert!(skill_is_codex_compatible(&valid));
        assert!(!skill_is_codex_compatible(&invalid));
    }

    #[test]
    fn test_skill_is_copilot_compatible() {
        let valid = make_skill(
            "valid",
            "---\nname: valid\ndescription: Test\n---\n# Content",
        );
        let invalid = make_skill("invalid", "# No frontmatter");

        assert!(skill_is_copilot_compatible(&valid));
        assert!(!skill_is_copilot_compatible(&invalid));
    }

    #[test]
    fn test_apply_autofix() {
        let skill = make_skill("my-skill", "# My Skill\nSome content here.");
        let fixed = apply_autofix_to_skill(&skill).unwrap();
        let fixed_str = String::from_utf8_lossy(&fixed);

        assert!(fixed_str.contains("---"));
        assert!(fixed_str.contains("name: my-skill"));
    }

    #[test]
    fn test_validation_report() {
        let skills = vec![
            make_skill(
                "valid",
                "---\nname: valid\ndescription: Test\n---\n# Content",
            ),
            make_skill("invalid", "# No frontmatter"),
        ];
        let options = SyncValidationOptions::default();
        let report = validate_skills_for_sync(&skills, ValidationTarget::Codex, &options);

        assert_eq!(report.total, 2);
        assert_eq!(report.passed, 1);
        assert_eq!(report.failed, 1);
        assert!(!report.all_valid());
    }
}
