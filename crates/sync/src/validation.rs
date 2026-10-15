//! Validation integration for skill sync.
//!
//! Provides validation and autofix capabilities during sync operations
//! to ensure skills are compatible with their target CLI.

use crate::common::Command;
use skrills_validate::{
    autofix_frontmatter, is_codex_compatible, is_copilot_compatible, validate_skill,
    AutofixOptions, AutofixResult, ValidationResult, ValidationTarget,
};
use tracing::warn;

/// Options for validation during sync.
#[derive(Debug, Clone, Default)]
pub struct SyncValidationOptions {
    /// Whether to auto-fix skills that fail validation.
    pub autofix: bool,
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

    let is_valid = valid_for(&validation, target);

    let mut autofix_result = None;
    let mut can_sync = is_valid;

    // If validation failed and autofix is enabled, try to fix. Non-UTF-8
    // content is never autofixed: the fix would work on a lossy copy.
    if !is_valid && options.autofix && std::str::from_utf8(&skill.content).is_ok() {
        let autofix_opts = AutofixOptions {
            // Nothing is written here, so there is nothing to back up.
            create_backup: false,
            write_changes: false,
            suggested_name: Some(skill.name.clone()),
            suggested_description: None,
        };

        match autofix_frontmatter(&skill.source_path, &content, &autofix_opts) {
            Ok(result) => {
                // A change is not a fix: the result must pass on its own.
                if result.modified {
                    let fixed = validate_skill(&skill.source_path, &result.content, target);
                    can_sync = valid_for(&fixed, target);
                }
                autofix_result = Some(result);
            }
            Err(e) => {
                warn!(
                    skill = %skill.name,
                    path = %skill.source_path.display(),
                    error = %e,
                    "Autofix failed for skill"
                );
            }
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

fn valid_for(validation: &ValidationResult, target: ValidationTarget) -> bool {
    match target {
        ValidationTarget::Claude => validation.is_claude_valid(),
        ValidationTarget::Codex => validation.is_codex_valid(),
        ValidationTarget::Copilot => validation.is_copilot_valid(),
        ValidationTarget::Both => validation.is_claude_valid() && validation.is_codex_valid(),
        ValidationTarget::All => validation.is_valid_for_all(),
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

        if result.validation.is_valid_for_all() {
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
///
/// Non-UTF-8 content is an error: fixing a lossy copy would replace every
/// invalid byte with U+FFFD.
pub fn apply_autofix_to_skill(skill: &Command) -> Result<Vec<u8>, String> {
    let content = std::str::from_utf8(&skill.content)
        .map_err(|e| format!("{} is not valid UTF-8: {e}", skill.source_path.display()))?;

    let options = AutofixOptions {
        create_backup: false,
        write_changes: false,
        suggested_name: Some(skill.name.clone()),
        suggested_description: None,
    };

    let result = autofix_frontmatter(&skill.source_path, content, &options)?;

    if result.modified {
        Ok(result.content.into_bytes())
    } else {
        Ok(skill.content.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::SystemTime;

    /// Autofix adds the missing name but keeps the empty description, yet any
    /// change used to mark the skill syncable.
    #[test]
    fn autofix_that_leaves_another_error_does_not_make_the_skill_syncable() {
        let skill = make_skill("demo", "---\ndescription: \"\"\n---\n# Body\n");
        let options = SyncValidationOptions { autofix: true };

        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        assert!(result.autofix.as_ref().is_some_and(|a| a.modified));
        assert!(!result.can_sync);
    }

    /// A lossy decode put U+FFFD in place of every invalid byte.
    #[test]
    fn apply_autofix_refuses_non_utf8_content() {
        let mut skill = make_skill("bin", "");
        skill.content = b"---\ndescription: d\n---\n\xFF\xFE body\n".to_vec();

        assert!(apply_autofix_to_skill(&skill).is_err());
    }

    /// Autofix must never see a lossy copy: the bytes it would hand back
    /// carry U+FFFD in place of every invalid byte.
    #[test]
    fn autofix_leaves_non_utf8_skill_bytes_untouched() {
        let mut skill = make_skill("bin", "");
        let original = b"# no frontmatter \xFF\xFE body\n".to_vec();
        skill.content = original.clone();
        let options = SyncValidationOptions { autofix: true };

        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        assert!(result.autofix.is_none());
        assert!(!result.can_sync);
        assert_eq!(skill.content, original);
    }

    fn make_skill(name: &str, content: &str) -> Command {
        let mut cmd = Command::new(
            name.to_string(),
            content.as_bytes().to_vec(),
            PathBuf::from(format!("{name}.md")),
        );
        cmd.modified = SystemTime::UNIX_EPOCH;
        cmd
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
        assert!(result.validation.is_codex_valid());
    }

    #[test]
    fn test_validate_invalid_skill_without_autofix() {
        let skill = make_skill("test", "# No frontmatter");
        let options = SyncValidationOptions::default();
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        assert!(!result.can_sync);
        assert!(!result.validation.is_codex_valid());
    }

    #[test]
    fn test_validate_invalid_skill_with_autofix() {
        let skill = make_skill("test", "# No frontmatter\nBut has content.");
        let options = SyncValidationOptions { autofix: true };
        let result = validate_skill_for_sync(&skill, ValidationTarget::Codex, &options);

        assert!(result.can_sync);
        let autofix = result
            .autofix
            .expect("autofix should be Some when autofix option is enabled");
        assert!(autofix.modified);
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
