//! Skill validation for Claude Code, Codex CLI, and GitHub Copilot CLI.
//!
//! Validates `SKILL.md` files against Claude Code (permissive), Codex CLI (strict),
//! and GitHub Copilot CLI (strict) requirements.
//!
//! # Example
//!
//! ```rust
//! use skrills_validate::{validate_skill, ValidationTarget};
//! use std::path::Path;
//!
//! let content = r#"---
//! name: my-skill
//! description: A helpful skill
//! ---
//! # My Skill
//! Content here.
//! "#;
//!
//! let result = validate_skill(Path::new("skill.md"), content, ValidationTarget::Both);
//! println!("Claude valid: {}", result.claude_valid);
//! println!("Codex valid: {}", result.codex_valid);
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Error type for validation operations.
pub type Error = anyhow::Error;
/// Result type for validation operations.
pub type Result<T> = std::result::Result<T, Error>;

pub mod autofix;
pub mod claude;
pub mod codex;
pub mod common;
pub mod copilot;
pub mod frontmatter;
#[cfg(feature = "watch")]
pub mod watch;

pub use autofix::{
    autofix_frontmatter, scaffold_body, to_kebab_case, AutofixOptions, AutofixResult,
};
pub use common::{Severity, ValidationIssue, ValidationResult, ValidationTarget};
pub use frontmatter::{
    generate_frontmatter, has_frontmatter, parse_frontmatter, ParsedSkill, SkillFrontmatter,
};

use std::path::Path;
use walkdir::WalkDir;

/// Merges multiple `ValidationResult`s into one, combining validity flags and
/// dropping only issues repeated for the same target, severity and message.
fn merge_validation_results(results: Vec<ValidationResult>, path: &Path) -> ValidationResult {
    let name = results
        .iter()
        .map(|r| r.name.clone())
        .max()
        .unwrap_or_default();

    let mut merged = ValidationResult::new(path.to_path_buf(), name);

    for result in results {
        merged.claude_valid &= result.claude_valid;
        merged.codex_valid &= result.codex_valid;
        merged.copilot_valid &= result.copilot_valid;

        for issue in result.issues {
            if !merged.issues.iter().any(|i| {
                i.target == issue.target
                    && i.severity == issue.severity
                    && i.message == issue.message
            }) {
                merged.issues.push(issue);
            }
        }
    }

    merged
}

/// Validates a single skill file.
pub fn validate_skill(path: &Path, content: &str, target: ValidationTarget) -> ValidationResult {
    let mut result = match target {
        ValidationTarget::Claude => claude::validate_claude(path, content),
        ValidationTarget::Codex => codex::validate_codex(path, content),
        ValidationTarget::Copilot => copilot::validate_copilot(path, content),
        ValidationTarget::Both => merge_validation_results(
            vec![
                claude::validate_claude(path, content),
                codex::validate_codex(path, content),
            ],
            path,
        ),
        ValidationTarget::All => merge_validation_results(
            vec![
                claude::validate_claude(path, content),
                codex::validate_codex(path, content),
                copilot::validate_copilot(path, content),
            ],
            path,
        ),
    };
    result.checked = Some(target);
    result
}

/// Validates all skills in a directory.
///
/// Recursively walks the directory looking for `SKILL.md` files,
/// skipping hidden directories and symlinks to match Codex discovery behavior.
pub fn validate_all(dir: &Path, target: ValidationTarget) -> Result<Vec<ValidationResult>> {
    if !dir.is_dir() {
        anyhow::bail!("skill directory not found: {}", dir.display());
    }
    let mut results = Vec::new();

    let is_hidden_rel_path = |path: &Path| {
        path.components().any(|c| match c {
            std::path::Component::Normal(s) => s.to_string_lossy().starts_with('.'),
            _ => false,
        })
    };

    for entry in WalkDir::new(dir)
        // Match Codex discovery behavior: skip symlinks.
        .follow_links(false)
        .into_iter()
        .filter_map(|e| match e {
            Ok(entry) => Some(entry),
            Err(err) => {
                tracing::warn!(
                    path = ?err.path(),
                    error = %err,
                    "Skipping directory entry during validation"
                );
                None
            }
        })
    {
        let path = entry.path();
        let rel = path.strip_prefix(dir).unwrap_or(path);
        if is_hidden_rel_path(rel) {
            continue;
        }

        // Only process SKILL.md files (Codex requires the filename to be exactly SKILL.md).
        // `entry.file_type()` describes the entry itself; `Path::is_file`
        // would follow a symlinked SKILL.md despite `follow_links(false)`.
        if entry.file_type().is_file() {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if filename == "SKILL.md" {
                match std::fs::read_to_string(path) {
                    Ok(content) => results.push(validate_skill(path, &content, target)),
                    // One unreadable file is reported as that skill's error
                    // rather than discarding every result collected so far.
                    Err(err) => {
                        let name = autofix::derive_name_from_path(path);
                        let mut unreadable = ValidationResult::new(path.to_path_buf(), name);
                        unreadable.add_issue(ValidationIssue::error(
                            ValidationTarget::All,
                            format!("Cannot read skill file: {err}"),
                        ));
                        unreadable.checked = Some(target);
                        results.push(unreadable);
                    }
                }
            }
        }
    }

    Ok(results)
}

/// Checks if a skill is Codex-compatible.
pub fn is_codex_compatible(content: &str) -> bool {
    codex::is_codex_compatible(content)
}

/// Checks if a skill is Copilot-compatible.
pub fn is_copilot_compatible(content: &str) -> bool {
    copilot::is_copilot_compatible(content)
}

/// Summary of validation results.
#[derive(Debug, Default)]
pub struct ValidationSummary {
    /// Total number of skills validated.
    pub total: usize,
    /// Number of skills valid for Claude Code.
    pub claude_valid: usize,
    /// Number of skills valid for Codex CLI.
    pub codex_valid: usize,
    /// Number of skills valid for GitHub Copilot CLI.
    pub copilot_valid: usize,
    /// Number of skills that pass validation for all targets simultaneously
    /// (intersection of claude_valid, codex_valid, and copilot_valid, over the
    /// targets each skill was checked against).
    pub all_valid: usize,
    /// Total number of error-level issues.
    pub error_count: usize,
    /// Total number of warning-level issues.
    pub warning_count: usize,
}

impl ValidationSummary {
    /// Creates a summary from validation results.
    pub fn from_results(results: &[ValidationResult]) -> Self {
        let mut summary = ValidationSummary {
            total: results.len(),
            ..Default::default()
        };

        for result in results {
            let claude = result.claude_valid && result.was_checked(ValidationTarget::Claude);
            let codex = result.codex_valid && result.was_checked(ValidationTarget::Codex);
            let copilot = result.copilot_valid && result.was_checked(ValidationTarget::Copilot);
            summary.claude_valid += usize::from(claude);
            summary.codex_valid += usize::from(codex);
            summary.copilot_valid += usize::from(copilot);
            // A target that was not checked does not block "all valid".
            if result.claude_valid && result.codex_valid && result.copilot_valid {
                summary.all_valid += 1;
            }
            summary.error_count += result.error_count();
            summary.warning_count += result.warning_count();
        }

        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_skill_all() {
        let content = "---\nname: test\ndescription: A test skill\n---\n# Content\nBody.";
        let result = validate_skill(Path::new("test.md"), content, ValidationTarget::All);

        assert!(result.claude_valid);
        assert!(result.codex_valid);
        assert!(result.copilot_valid);
    }

    #[test]
    fn test_validate_skill_claude_only() {
        let content = "# Just markdown\nNo frontmatter.";
        let result = validate_skill(Path::new("test.md"), content, ValidationTarget::All);

        assert!(result.claude_valid);
        assert!(!result.codex_valid);
        assert!(!result.copilot_valid);
    }

    #[test]
    fn test_validate_skill_copilot() {
        let content = "---\nname: test\ndescription: A test skill\n---\n# Content\nBody.";
        let result = validate_skill(Path::new("test.md"), content, ValidationTarget::Copilot);

        assert!(result.copilot_valid);
    }

    #[test]
    fn test_validation_summary() {
        let results = vec![
            ValidationResult {
                path: "a.md".into(),
                name: "a".into(),
                issues: vec![],
                claude_valid: true,
                codex_valid: true,
                copilot_valid: true,
                checked: None,
            },
            ValidationResult {
                path: "b.md".into(),
                name: "b".into(),
                issues: vec![ValidationIssue::error(ValidationTarget::Codex, "test")],
                claude_valid: true,
                codex_valid: false,
                copilot_valid: true,
                checked: None,
            },
            ValidationResult {
                path: "c.md".into(),
                name: "c".into(),
                issues: vec![ValidationIssue::error(ValidationTarget::Copilot, "test")],
                claude_valid: true,
                codex_valid: true,
                copilot_valid: false,
                checked: None,
            },
        ];

        let summary = ValidationSummary::from_results(&results);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.claude_valid, 3);
        assert_eq!(summary.codex_valid, 2);
        assert_eq!(summary.copilot_valid, 2);
        assert_eq!(summary.all_valid, 1);
    }

    #[test]
    fn merge_keeps_each_targets_issue_when_messages_match() {
        // No frontmatter: Codex and Copilot both report the missing name and
        // description; each target's issues must survive the merge.
        let content = "---\nlicense: MIT\n---\n# Body\nText.";
        let all = validate_skill(Path::new("SKILL.md"), content, ValidationTarget::All);
        let codex = validate_skill(Path::new("SKILL.md"), content, ValidationTarget::Codex);
        let copilot = validate_skill(Path::new("SKILL.md"), content, ValidationTarget::Copilot);
        let copilot_issues = all
            .issues
            .iter()
            .filter(|i| i.target == ValidationTarget::Copilot)
            .count();
        assert_eq!(copilot_issues, copilot.issues.len());
        assert_eq!(
            all.error_count(),
            codex.error_count() + copilot.error_count()
        );
    }

    #[test]
    fn summary_does_not_count_copilot_valid_when_copilot_was_not_checked() {
        let content = "---\nname: test\ndescription: A test skill\n---\n# Content\nBody.";
        let both = validate_skill(Path::new("SKILL.md"), content, ValidationTarget::Both);
        let summary = ValidationSummary::from_results(&[both]);
        assert_eq!(summary.claude_valid, 1);
        assert_eq!(summary.codex_valid, 1);
        assert_eq!(summary.copilot_valid, 0);
        assert_eq!(
            summary.all_valid, 1,
            "valid for every target it was checked against"
        );
    }

    #[cfg(unix)]
    #[test]
    fn validate_all_reports_an_unreadable_skill_and_keeps_the_others() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let good = temp_dir.path().join("good");
        let bad = temp_dir.path().join("bad");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(
            good.join("SKILL.md"),
            "---\nname: good\ndescription: fine\n---\n# Content\nBody.",
        )
        .unwrap();
        std::fs::write(bad.join("SKILL.md"), [0xff, 0xfe, 0x00, 0x80]).unwrap();

        let results = validate_all(temp_dir.path(), ValidationTarget::All).unwrap();
        assert_eq!(results.len(), 2);
        let bad_result = results
            .iter()
            .find(|r| r.path.starts_with(&bad))
            .expect("unreadable skill reported");
        assert!(bad_result.has_errors());
        assert!(!bad_result.claude_valid && !bad_result.codex_valid && !bad_result.copilot_valid);
        assert!(results
            .iter()
            .any(|r| r.path.starts_with(&good) && r.codex_valid));
    }

    /// Nothing in the workspace calls `watch::run_watch_loop`, so its notify
    /// and libc dependencies (and its process-wide SIGINT handler) stay opt-in.
    #[test]
    fn watch_feature_is_not_enabled_by_default() {
        let manifest = include_str!("../Cargo.toml");
        let default_line = manifest
            .lines()
            .find(|l| l.trim_start().starts_with("default ="))
            .expect("Cargo.toml declares default features");
        assert!(!default_line.contains("watch"), "{default_line}");
    }

    /// BDD Test: Validate all skills in a directory
    ///
    /// Given: A directory containing multiple SKILL.md files
    /// When: validate_all is called with the directory path
    /// Then: All skills are validated and results returned
    #[test]
    fn given_skill_directory_when_validate_all_then_returns_all_results() {
        use tempfile::TempDir;

        // GIVEN: A directory with multiple skill files
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path();

        // Create valid skill
        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: valid-skill\ndescription: A valid skill\n---\n# Content\nBody.",
        )
        .unwrap();

        // Create subdirectory with another skill
        let subdir = skills_dir.join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(
            subdir.join("SKILL.md"),
            "---\nname: sub-skill\ndescription: Another skill\n---\n# Content\nBody.",
        )
        .unwrap();

        // WHEN: Validating all skills in the directory
        let results = validate_all(skills_dir, ValidationTarget::Both).unwrap();

        // THEN: All skill files are found and validated
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.claude_valid && r.codex_valid));
    }

    /// BDD Test: Hidden directories are skipped during validation
    ///
    /// Given: A directory with hidden subdirectories containing skills
    /// When: validate_all is called
    /// Then: Skills in hidden directories are skipped
    #[test]
    fn given_hidden_directory_when_validate_all_then_skips_hidden() {
        use tempfile::TempDir;

        // GIVEN: A directory with a hidden subdirectory containing a skill
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path();

        let hidden_dir = skills_dir.join(".hidden");
        std::fs::create_dir(&hidden_dir).unwrap();
        std::fs::write(
            hidden_dir.join("SKILL.md"),
            "---\nname: hidden-skill\ndescription: Should be skipped\n---\n# Content\nBody.",
        )
        .unwrap();

        // Create a visible skill
        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: visible-skill\ndescription: Should be found\n---\n# Content\nBody.",
        )
        .unwrap();

        // WHEN: Validating all skills
        let results = validate_all(skills_dir, ValidationTarget::Both).unwrap();

        // THEN: Only visible skill is validated
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "visible-skill");
    }

    /// BDD Test: Only SKILL.md files are processed
    ///
    /// Given: A directory with various markdown files
    /// When: validate_all is called
    /// Then: Only files named exactly SKILL.md are validated
    #[test]
    fn given_mixed_files_when_validate_all_then_only_skill_md() {
        use tempfile::TempDir;

        // GIVEN: A directory with different markdown files
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path();

        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: skill-md\ndescription: Valid name\n---\n# Content\nBody.",
        )
        .unwrap();

        std::fs::write(
            skills_dir.join("Skills.md"), // wrong casing - should be ignored
            "---\nname: wrong-case\ndescription: Should be ignored\n---\n# Content\nBody.",
        )
        .unwrap();

        std::fs::write(
            skills_dir.join("README.md"), // different name - should be ignored
            "# README\nNot a skill file.",
        )
        .unwrap();

        // WHEN: Validating all skills
        let results = validate_all(skills_dir, ValidationTarget::Both).unwrap();

        // THEN: Only SKILL.md is processed
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "skill-md");
    }

    /// BDD Test: Empty directory returns empty results
    ///
    /// Given: An empty directory
    /// When: validate_all is called
    /// Then: Returns empty vector without error
    #[test]
    fn given_empty_directory_when_validate_all_then_empty_results() {
        use tempfile::TempDir;

        // GIVEN: An empty directory
        let temp_dir = TempDir::new().unwrap();

        // WHEN: Validating all skills
        let results = validate_all(temp_dir.path(), ValidationTarget::Both).unwrap();

        // THEN: No results returned
        assert_eq!(results.len(), 0);
        assert!(results.is_empty());
    }

    /// BDD Test: validate_all handles invalid skills gracefully
    ///
    /// Given: A directory with invalid skill files
    /// When: validate_all is called
    /// Then: Returns validation results with errors
    #[test]
    fn given_invalid_skills_when_validate_all_then_includes_errors() {
        use tempfile::TempDir;

        // GIVEN: A directory with invalid skill (missing required fields)
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path();

        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: invalid-skill\n---\n# Content\nMissing description for Codex/Copilot.",
        )
        .unwrap();

        // WHEN: Validating with strict target
        let results = validate_all(skills_dir, ValidationTarget::Codex).unwrap();

        // THEN: Validation errors are included
        assert_eq!(results.len(), 1);
        assert!(!results[0].codex_valid);
        assert!(!results[0].issues.is_empty());
    }

    /// BDD Test: validate_all with All target validates against all platforms
    ///
    /// Given: A valid skill file
    /// When: validate_all is called with ValidationTarget::All
    /// Then: Result includes validation status for all three platforms
    #[test]
    fn given_valid_skill_when_validate_all_targets_then_all_valid() {
        use tempfile::TempDir;

        // GIVEN: A valid skill file
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path();

        std::fs::write(
            skills_dir.join("SKILL.md"),
            "---\nname: multi-skill\ndescription: Works everywhere\n---\n# Content\nBody.",
        )
        .unwrap();

        // WHEN: Validating against all targets
        let results = validate_all(skills_dir, ValidationTarget::All).unwrap();

        // THEN: All platforms marked as valid
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert!(result.claude_valid);
        assert!(result.codex_valid);
        assert!(result.copilot_valid);
    }

    /// BDD Test: Partially valid skill produces mixed results
    ///
    /// Given: A skill valid only for Claude
    /// When: validate_all is called with ValidationTarget::All
    /// Then: Result shows Claude valid but Codex/Copilot invalid
    #[test]
    fn given_claude_only_skill_when_validate_all_then_partial_validity() {
        use tempfile::TempDir;

        // GIVEN: A markdown-only skill (no frontmatter)
        let temp_dir = TempDir::new().unwrap();
        let skills_dir = temp_dir.path();

        std::fs::write(
            skills_dir.join("SKILL.md"),
            "# Claude Only\n\nThis skill has no frontmatter.",
        )
        .unwrap();

        // WHEN: Validating against all targets
        let results = validate_all(skills_dir, ValidationTarget::All).unwrap();

        // THEN: Mixed validity results
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert!(result.claude_valid);
        assert!(!result.codex_valid);
        assert!(!result.copilot_valid);
    }
}

// === Error path tests ===

#[test]
fn test_validate_empty_content() {
    let result = validate_skill(Path::new("empty.md"), "", ValidationTarget::All);
    // Empty content fails all validators
    assert!(!result.codex_valid);
    assert!(!result.copilot_valid);
    // Has validation issues
    assert!(!result.issues.is_empty() || !result.claude_valid);
}

#[test]
fn test_validate_no_frontmatter_delimiter() {
    let content = "Just plain text without any YAML frontmatter delimiters.";
    let result = validate_skill(Path::new("no-fm.md"), content, ValidationTarget::All);
    assert!(result.claude_valid);
    assert!(!result.codex_valid);
    assert!(!result.copilot_valid);
}

#[test]
fn test_validate_malformed_frontmatter_missing_description() {
    let content = "---\nname: test-skill\n---\n# Content\nBody.";
    let result = validate_skill(Path::new("no-desc.md"), content, ValidationTarget::Codex);
    assert!(!result.codex_valid);
    assert!(!result.issues.is_empty());
}

#[test]
fn test_validate_malformed_frontmatter_missing_name() {
    let content = "---\ndescription: A skill without a name\n---\n# Content\nBody.";
    let result = validate_skill(Path::new("no-name.md"), content, ValidationTarget::Codex);
    assert!(!result.codex_valid);
}

#[test]
fn test_validate_frontmatter_empty_yaml_block() {
    let content = "---\n---\n# Content\nBody.";
    let result = validate_skill(Path::new("empty-fm.md"), content, ValidationTarget::All);
    assert!(result.claude_valid);
    assert!(!result.codex_valid);
    assert!(!result.copilot_valid);
}

#[test]
fn test_validate_all_nonexistent_directory() {
    let result = validate_all(
        Path::new("/nonexistent/path/that/does/not/exist"),
        ValidationTarget::Both,
    );
    // A mistyped root must not read as "0 skills, all valid".
    assert!(result.is_err());
}

#[test]
fn test_validate_both_target_merges_issues() {
    // Content valid for Claude but not Codex (missing description)
    let content = "---\nname: merge-test\n---\n# Content\nBody.";
    let result = validate_skill(Path::new("merge.md"), content, ValidationTarget::Both);
    assert!(result.claude_valid);
    assert!(!result.codex_valid);
    // Should have issues from Codex validation
    assert!(!result.issues.is_empty());
}

#[test]
fn test_validation_summary_empty_results() {
    let summary = ValidationSummary::from_results(&[]);
    assert_eq!(summary.total, 0);
    assert_eq!(summary.claude_valid, 0);
    assert_eq!(summary.codex_valid, 0);
    assert_eq!(summary.copilot_valid, 0);
    assert_eq!(summary.all_valid, 0);
    assert_eq!(summary.error_count, 0);
    assert_eq!(summary.warning_count, 0);
}

#[test]
fn test_validate_frontmatter_invalid_yaml_syntax() {
    let content = "---\nname: [unclosed bracket\n---\n# Content";
    let result = validate_skill(Path::new("bad-yaml.md"), content, ValidationTarget::Codex);
    // Should handle gracefully - either invalid or has issues
    assert!(!result.codex_valid || !result.issues.is_empty());
}

/// `Path::is_file` follows links, so a synced link was validated twice.
#[cfg(unix)]
#[test]
fn validate_all_skips_symlinked_skill_files() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().unwrap();
    let real = temp_dir.path().join("real");
    let linked = temp_dir.path().join("linked");
    std::fs::create_dir_all(&real).unwrap();
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(
        real.join("SKILL.md"),
        "---\nname: real\ndescription: fine\n---\n# Content\nBody.",
    )
    .unwrap();
    std::os::unix::fs::symlink(real.join("SKILL.md"), linked.join("SKILL.md")).unwrap();

    let results = validate_all(temp_dir.path(), ValidationTarget::All).unwrap();
    assert_eq!(results.len(), 1, "{results:?}");
}
