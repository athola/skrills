//! Common types for skill validation.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Severity level for validation issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Severity {
    /// Critical issue that prevents skill from working.
    Error,
    /// Issue that may cause problems but skill can still load.
    Warning,
    /// Suggestion for improvement.
    Info,
}

/// Target CLI for validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationTarget {
    /// Claude Code validation (permissive).
    Claude,
    /// Codex CLI validation (strict, requires frontmatter).
    Codex,
    /// Both targets.
    Both,
}

/// A single validation issue found in a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    /// Severity of the issue.
    pub severity: Severity,
    /// Target CLI affected.
    pub target: ValidationTarget,
    /// Human-readable message.
    pub message: String,
    /// Line number in the file (1-indexed), if applicable.
    pub line: Option<usize>,
    /// Suggested fix, if available.
    pub suggestion: Option<String>,
}

impl ValidationIssue {
    /// Create an error-level issue.
    pub fn error(target: ValidationTarget, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            target,
            message: message.into(),
            line: None,
            suggestion: None,
        }
    }

    /// Create a warning-level issue.
    pub fn warning(target: ValidationTarget, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            target,
            message: message.into(),
            line: None,
            suggestion: None,
        }
    }

    /// Create an info-level issue.
    pub fn info(target: ValidationTarget, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Info,
            target,
            message: message.into(),
            line: None,
            suggestion: None,
        }
    }

    /// Add a line number to the issue.
    pub fn with_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }

    /// Add a suggested fix to the issue.
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }
}

/// Result of validating a single skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Path to the skill file.
    pub path: PathBuf,
    /// Skill name (from frontmatter or filename).
    pub name: String,
    /// Issues found during validation.
    pub issues: Vec<ValidationIssue>,
    /// Whether the skill is valid for Claude Code.
    pub claude_valid: bool,
    /// Whether the skill is valid for Codex CLI.
    pub codex_valid: bool,
}

impl ValidationResult {
    /// Create a new validation result with no issues.
    pub fn new(path: PathBuf, name: String) -> Self {
        Self {
            path,
            name,
            issues: Vec::new(),
            claude_valid: true,
            codex_valid: true,
        }
    }

    /// Add an issue to the result, updating validity flags if it's an error.
    pub fn add_issue(&mut self, issue: ValidationIssue) {
        if issue.severity == Severity::Error {
            match issue.target {
                ValidationTarget::Claude => self.claude_valid = false,
                ValidationTarget::Codex => self.codex_valid = false,
                ValidationTarget::Both => {
                    self.claude_valid = false;
                    self.codex_valid = false;
                }
            }
        }
        self.issues.push(issue);
    }

    /// Returns true if there are any error-level issues.
    pub fn has_errors(&self) -> bool {
        self.issues.iter().any(|i| i.severity == Severity::Error)
    }

    /// Returns the number of error-level issues.
    pub fn error_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Error)
            .count()
    }

    /// Returns the number of warning-level issues.
    pub fn warning_count(&self) -> usize {
        self.issues
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================
    // ValidationIssue Tests (BDD style)
    // ==========================================

    mod validation_issue {
        use super::*;

        #[test]
        fn given_error_level_when_created_then_severity_is_error() {
            // Given/When
            let issue = ValidationIssue::error(ValidationTarget::Claude, "test error");

            // Then
            assert_eq!(issue.severity, Severity::Error);
            assert_eq!(issue.target, ValidationTarget::Claude);
            assert_eq!(issue.message, "test error");
            assert!(issue.line.is_none());
            assert!(issue.suggestion.is_none());
        }

        #[test]
        fn given_warning_level_when_created_then_severity_is_warning() {
            let issue = ValidationIssue::warning(ValidationTarget::Codex, "test warning");

            assert_eq!(issue.severity, Severity::Warning);
            assert_eq!(issue.target, ValidationTarget::Codex);
            assert_eq!(issue.message, "test warning");
        }

        #[test]
        fn given_info_level_when_created_then_severity_is_info() {
            let issue = ValidationIssue::info(ValidationTarget::Both, "test info");

            assert_eq!(issue.severity, Severity::Info);
            assert_eq!(issue.target, ValidationTarget::Both);
            assert_eq!(issue.message, "test info");
        }

        #[test]
        fn when_with_line_called_then_line_is_set() {
            let issue = ValidationIssue::error(ValidationTarget::Claude, "msg").with_line(42);

            assert_eq!(issue.line, Some(42));
        }

        #[test]
        fn when_with_suggestion_called_then_suggestion_is_set() {
            let issue =
                ValidationIssue::error(ValidationTarget::Claude, "msg").with_suggestion("fix it");

            assert_eq!(issue.suggestion, Some("fix it".to_string()));
        }

        #[test]
        fn when_chained_builders_then_all_fields_set() {
            let issue = ValidationIssue::warning(ValidationTarget::Both, "chained")
                .with_line(10)
                .with_suggestion("do this");

            assert_eq!(issue.severity, Severity::Warning);
            assert_eq!(issue.line, Some(10));
            assert_eq!(issue.suggestion, Some("do this".to_string()));
        }
    }

    // ==========================================
    // ValidationResult Tests (BDD style)
    // ==========================================

    mod validation_result {
        use super::*;

        #[test]
        fn given_new_result_when_created_then_valid_for_both_targets() {
            let result = ValidationResult::new("test.md".into(), "test".to_string());

            assert!(result.claude_valid);
            assert!(result.codex_valid);
            assert!(result.issues.is_empty());
            assert_eq!(result.name, "test");
        }

        #[test]
        fn given_result_when_claude_error_added_then_claude_invalid() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());

            result.add_issue(ValidationIssue::error(
                ValidationTarget::Claude,
                "claude issue",
            ));

            assert!(!result.claude_valid);
            assert!(result.codex_valid);
        }

        #[test]
        fn given_result_when_codex_error_added_then_codex_invalid() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());

            result.add_issue(ValidationIssue::error(
                ValidationTarget::Codex,
                "codex issue",
            ));

            assert!(result.claude_valid);
            assert!(!result.codex_valid);
        }

        #[test]
        fn given_result_when_both_error_added_then_both_invalid() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());

            result.add_issue(ValidationIssue::error(ValidationTarget::Both, "both issue"));

            assert!(!result.claude_valid);
            assert!(!result.codex_valid);
        }

        #[test]
        fn given_result_when_warning_added_then_still_valid() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());

            result.add_issue(ValidationIssue::warning(
                ValidationTarget::Both,
                "just a warning",
            ));

            assert!(result.claude_valid);
            assert!(result.codex_valid);
        }

        #[test]
        fn given_result_when_info_added_then_still_valid() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());

            result.add_issue(ValidationIssue::info(ValidationTarget::Both, "just info"));

            assert!(result.claude_valid);
            assert!(result.codex_valid);
        }

        #[test]
        fn given_result_with_errors_when_has_errors_then_true() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());
            result.add_issue(ValidationIssue::error(ValidationTarget::Claude, "an error"));

            assert!(result.has_errors());
        }

        #[test]
        fn given_result_with_no_errors_when_has_errors_then_false() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());
            result.add_issue(ValidationIssue::warning(
                ValidationTarget::Claude,
                "a warning",
            ));

            assert!(!result.has_errors());
        }

        #[test]
        fn given_mixed_issues_when_error_count_then_counts_only_errors() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());
            result.add_issue(ValidationIssue::error(ValidationTarget::Claude, "error 1"));
            result.add_issue(ValidationIssue::warning(
                ValidationTarget::Claude,
                "warning",
            ));
            result.add_issue(ValidationIssue::error(ValidationTarget::Codex, "error 2"));
            result.add_issue(ValidationIssue::info(ValidationTarget::Both, "info"));

            assert_eq!(result.error_count(), 2);
        }

        #[test]
        fn given_mixed_issues_when_warning_count_then_counts_only_warnings() {
            let mut result = ValidationResult::new("test.md".into(), "test".to_string());
            result.add_issue(ValidationIssue::error(ValidationTarget::Claude, "error"));
            result.add_issue(ValidationIssue::warning(
                ValidationTarget::Claude,
                "warning 1",
            ));
            result.add_issue(ValidationIssue::warning(
                ValidationTarget::Codex,
                "warning 2",
            ));
            result.add_issue(ValidationIssue::warning(
                ValidationTarget::Both,
                "warning 3",
            ));
            result.add_issue(ValidationIssue::info(ValidationTarget::Both, "info"));

            assert_eq!(result.warning_count(), 3);
        }

        #[test]
        fn given_empty_result_when_counts_then_zero() {
            let result = ValidationResult::new("test.md".into(), "test".to_string());

            assert_eq!(result.error_count(), 0);
            assert_eq!(result.warning_count(), 0);
            assert!(!result.has_errors());
        }
    }

    // ==========================================
    // Severity and ValidationTarget Tests
    // ==========================================

    mod severity_and_target {
        use super::*;

        #[test]
        fn severity_equality_works() {
            assert_eq!(Severity::Error, Severity::Error);
            assert_ne!(Severity::Error, Severity::Warning);
            assert_ne!(Severity::Warning, Severity::Info);
        }

        #[test]
        fn validation_target_equality_works() {
            assert_eq!(ValidationTarget::Claude, ValidationTarget::Claude);
            assert_ne!(ValidationTarget::Claude, ValidationTarget::Codex);
            assert_ne!(ValidationTarget::Codex, ValidationTarget::Both);
        }
    }
}
