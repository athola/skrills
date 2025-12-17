//! Skill validation for Claude Code and Codex CLI.
//!
//! This crate provides validation for SKILL.md files against the requirements
//! of both Claude Code (permissive) and Codex CLI (strict).
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

pub mod autofix;
pub mod claude;
pub mod codex;
pub mod common;
pub mod frontmatter;

pub use autofix::{autofix_frontmatter, AutofixOptions, AutofixResult};
pub use common::{Severity, ValidationIssue, ValidationResult, ValidationTarget};
pub use frontmatter::{
    generate_frontmatter, has_frontmatter, parse_frontmatter, ParsedSkill, SkillFrontmatter,
};

use std::path::Path;
use walkdir::WalkDir;

/// Validate a single skill file.
pub fn validate_skill(path: &Path, content: &str, target: ValidationTarget) -> ValidationResult {
    match target {
        ValidationTarget::Claude => claude::validate_claude(path, content),
        ValidationTarget::Codex => codex::validate_codex(path, content),
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
    }
}

/// Validate all skills in a directory.
pub fn validate_all(
    dir: &Path,
    target: ValidationTarget,
) -> Result<Vec<ValidationResult>, std::io::Error> {
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

/// Quick check if a skill is Codex-compatible.
pub fn is_codex_compatible(content: &str) -> bool {
    codex::is_codex_compatible(content)
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
    /// Number of skills valid for both.
    pub both_valid: usize,
    /// Total number of error-level issues.
    pub error_count: usize,
    /// Total number of warning-level issues.
    pub warning_count: usize,
}

impl ValidationSummary {
    /// Create a summary from validation results.
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
            if result.claude_valid && result.codex_valid {
                summary.both_valid += 1;
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
    fn test_validate_skill_both() {
        let content = "---\nname: test\ndescription: A test skill\n---\n# Content\nBody.";
        let result = validate_skill(Path::new("test.md"), content, ValidationTarget::Both);

        assert!(result.claude_valid);
        assert!(result.codex_valid);
    }

    #[test]
    fn test_validate_skill_claude_only() {
        let content = "# Just markdown\nNo frontmatter.";
        let result = validate_skill(Path::new("test.md"), content, ValidationTarget::Both);

        assert!(result.claude_valid);
        assert!(!result.codex_valid);
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
            },
            ValidationResult {
                path: "b.md".into(),
                name: "b".into(),
                issues: vec![ValidationIssue::error(ValidationTarget::Codex, "test")],
                claude_valid: true,
                codex_valid: false,
            },
        ];

        let summary = ValidationSummary::from_results(&results);
        assert_eq!(summary.total, 2);
        assert_eq!(summary.claude_valid, 2);
        assert_eq!(summary.codex_valid, 1);
        assert_eq!(summary.both_valid, 1);
    }
}
