//! Skill validation for Claude Code and Codex CLI.
//!
//! Validates `SKILL.md` files against Claude Code (permissive), Codex CLI (strict), and GitHub Copilot CLI (strict) requirements.
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

pub use autofix::{autofix_frontmatter, AutofixOptions, AutofixResult};
pub use common::{Severity, ValidationIssue, ValidationResult, ValidationTarget};
pub use frontmatter::{
    generate_frontmatter, has_frontmatter, parse_frontmatter, ParsedSkill, SkillFrontmatter,
};

use std::path::Path;
use walkdir::WalkDir;

/// Validates a single skill file.
pub fn validate_skill(path: &Path, content: &str, target: ValidationTarget) -> ValidationResult {
    match target {
        ValidationTarget::Claude => claude::validate_claude(path, content),
        ValidationTarget::Codex => codex::validate_codex(path, content),
        ValidationTarget::Copilot => copilot::validate_copilot(path, content),
        ValidationTarget::Both => {
            let claude_result = claude::validate_claude(path, content);
            let codex_result = codex::validate_codex(path, content);

            // Merge results
            let mut merged = ValidationResult::new(
                path.to_path_buf(),
                codex_result.name.clone().max(claude_result.name.clone()),
            );

            merged.claude_valid = claude_result.claude_valid;
            merged.codex_valid = codex_result.codex_valid;

            // Add unique issues from both
            for issue in claude_result.issues {
                merged.issues.push(issue);
            }
            for issue in codex_result.issues {
                // Avoid duplicates for shared issues
                if !merged.issues.iter().any(|i| i.message == issue.message) {
                    merged.issues.push(issue);
                }
            }

            merged
        }
        ValidationTarget::All => {
            let claude_result = claude::validate_claude(path, content);
            let codex_result = codex::validate_codex(path, content);
            let copilot_result = copilot::validate_copilot(path, content);

            // Merge results from all three
            let mut merged = ValidationResult::new(
                path.to_path_buf(),
                codex_result
                    .name
                    .clone()
                    .max(claude_result.name.clone())
                    .max(copilot_result.name.clone()),
            );

            merged.claude_valid = claude_result.claude_valid;
            merged.codex_valid = codex_result.codex_valid;
            merged.copilot_valid = copilot_result.copilot_valid;

            // Add unique issues from all three
            for issue in claude_result.issues {
                merged.issues.push(issue);
            }
            for issue in codex_result.issues {
                if !merged.issues.iter().any(|i| i.message == issue.message) {
                    merged.issues.push(issue);
                }
            }
            for issue in copilot_result.issues {
                if !merged.issues.iter().any(|i| i.message == issue.message) {
                    merged.issues.push(issue);
                }
            }

            merged
        }
    }
}

/// Validates all skills in a directory.
///
/// Recursively walks the directory looking for `SKILL.md` files,
/// skipping hidden directories and symlinks to match Codex discovery behavior.
pub fn validate_all(dir: &Path, target: ValidationTarget) -> Result<Vec<ValidationResult>> {
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
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        let rel = path.strip_prefix(dir).unwrap_or(path);
        if is_hidden_rel_path(rel) {
            continue;
        }

        // Only process SKILL.md files (Codex requires the filename to be exactly SKILL.md).
        if path.is_file() {
            let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if filename == "SKILL.md" {
                let content = std::fs::read_to_string(path)?;
                results.push(validate_skill(path, &content, target));
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
    /// Number of skills valid for all targets (Claude, Codex, and Copilot).
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
            if result.claude_valid {
                summary.claude_valid += 1;
            }
            if result.codex_valid {
                summary.codex_valid += 1;
            }
            if result.copilot_valid {
                summary.copilot_valid += 1;
            }
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
            },
            ValidationResult {
                path: "b.md".into(),
                name: "b".into(),
                issues: vec![ValidationIssue::error(ValidationTarget::Codex, "test")],
                claude_valid: true,
                codex_valid: false,
                copilot_valid: true,
            },
            ValidationResult {
                path: "c.md".into(),
                name: "c".into(),
                issues: vec![ValidationIssue::error(ValidationTarget::Copilot, "test")],
                claude_valid: true,
                codex_valid: true,
                copilot_valid: false,
            },
        ];

        let summary = ValidationSummary::from_results(&results);
        assert_eq!(summary.total, 3);
        assert_eq!(summary.claude_valid, 3);
        assert_eq!(summary.codex_valid, 2);
        assert_eq!(summary.copilot_valid, 2);
        assert_eq!(summary.all_valid, 1);
    }
}
