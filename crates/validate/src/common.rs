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
