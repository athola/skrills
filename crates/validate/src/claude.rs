//! Claude Code skill validation.
//!
//! Claude Code is permissive with skill files:
//! - Frontmatter is optional
//! - Name/description not strictly required
//! - Focuses on content quality

use crate::common::{ValidationIssue, ValidationResult, ValidationTarget};
use crate::frontmatter::{parse_frontmatter, ParsedSkill};
use std::path::Path;

/// Maximum recommended skill size for Claude Code (64KB).
const MAX_RECOMMENDED_SIZE: usize = 64 * 1024;

/// Validate a skill for Claude Code compatibility.
pub fn validate_claude(path: &Path, content: &str) -> ValidationResult {
    let name = crate::autofix::derive_name_from_path(path);

    let mut result = ValidationResult::new(path.to_path_buf(), name.clone());

    // Parse frontmatter (optional for Claude)
    let parsed = match parse_frontmatter(content) {
        Ok(p) => p,
        Err(e) => {
            result.add_issue(
                ValidationIssue::error(
                    ValidationTarget::Claude,
                    format!("Invalid frontmatter: {e}"),
                )
                .with_line(1),
            );
            return result;
        }
    };

    // Update name from frontmatter if available
    if let Some(ref fm) = parsed.frontmatter {
        if let Some(ref n) = fm.name {
            result.name = n.clone();
        }
    }

    validate_claude_content(&mut result, &parsed, content.len());

    result
}

fn validate_claude_content(result: &mut ValidationResult, parsed: &ParsedSkill, total_size: usize) {
    // Check file size
    if total_size > MAX_RECOMMENDED_SIZE {
        result.add_issue(
            ValidationIssue::warning(
                ValidationTarget::Claude,
                format!(
                    "Skill size ({} bytes) exceeds recommended maximum ({} bytes)",
                    total_size, MAX_RECOMMENDED_SIZE
                ),
            )
            .with_suggestion("Consider splitting into smaller, focused skills"),
        );
    }

    // Check for empty content
    if parsed.content.trim().is_empty() {
        result.add_issue(ValidationIssue::error(
            ValidationTarget::Claude,
            "Skill has no content",
        ));
    }

    // Info: suggest adding frontmatter for better organization
    if parsed.frontmatter.is_none() {
        result.add_issue(
            ValidationIssue::info(ValidationTarget::Claude, "No frontmatter present")
                .with_suggestion(
                    "Adding frontmatter (name, description) improves skill discoverability",
                ),
        );
    } else if let Some(ref fm) = parsed.frontmatter {
        // Check for missing recommended fields
        if fm.name.is_none() {
            result.add_issue(
                ValidationIssue::info(ValidationTarget::Claude, "Frontmatter missing 'name' field")
                    .with_suggestion("Add a 'name' field for better identification"),
            );
        }
        if fm.description.is_none() {
            result.add_issue(
                ValidationIssue::info(
                    ValidationTarget::Claude,
                    "Frontmatter missing 'description' field",
                )
                .with_suggestion("Add a 'description' field for better discoverability"),
            );
        }
    }

    // Check for common markdown issues
    check_markdown_quality(result, &parsed.content, parsed.content_start_line);
}

fn check_markdown_quality(result: &mut ValidationResult, content: &str, start_line: usize) {
    // Check for very long lines (might indicate formatting issues)
    for (i, line) in content.lines().enumerate() {
        let chars = line.chars().count();
        if chars > 500 && !line.starts_with("```") && !line.starts_with("    ") {
            result.add_issue(
                ValidationIssue::info(
                    ValidationTarget::Claude,
                    format!(
                        "Very long line ({} chars) may indicate formatting issues",
                        chars
                    ),
                )
                .with_line(start_line + i),
            );
            break; // Only report once
        }
    }

    // Check for unclosed code blocks: only a line that starts with ``` is a
    // fence; triple backticks inside a sentence are inline code.
    let code_fence_count = content
        .lines()
        .filter(|line| line.trim_start().starts_with("```"))
        .count();
    if !code_fence_count.is_multiple_of(2) {
        result.add_issue(
            ValidationIssue::warning(
                ValidationTarget::Claude,
                "Unclosed code block detected (odd number of ``` markers)",
            )
            .with_suggestion("Ensure all code blocks have opening and closing ``` markers"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Severity;
    use std::path::PathBuf;

    #[test]
    fn test_valid_skill() {
        let content = "---\nname: test\ndescription: A test skill\n---\n# Test\nSome content.";
        let result = validate_claude(&PathBuf::from("test.md"), content);

        assert!(result.claude_valid);
        assert_eq!(result.name, "test");
    }

    #[test]
    fn test_no_frontmatter() {
        let content = "# Just Content\nNo frontmatter here.";
        let result = validate_claude(&PathBuf::from("skill.md"), content);

        // Valid but with info suggestion
        assert!(result.claude_valid);
        assert!(result.issues.iter().any(|i| i.severity == Severity::Info));
    }

    #[test]
    fn test_empty_content() {
        let content = "---\nname: empty\n---\n";
        let result = validate_claude(&PathBuf::from("empty.md"), content);

        assert!(!result.claude_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("no content")));
    }

    #[test]
    fn test_unclosed_code_block() {
        let content = "# Skill\n```rust\nfn main() {}\n// missing closing fence";
        let result = validate_claude(&PathBuf::from("code.md"), content);

        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("Unclosed code block")));
    }

    #[test]
    fn inline_triple_backticks_do_not_count_as_a_fence() {
        let content = "# T\nUse ```inline``` code and ``` again.\n```\ncode\n```\n";
        let result = validate_claude(Path::new("x/SKILL.md"), content);
        assert!(
            !result
                .issues
                .iter()
                .any(|i| i.message.contains("Unclosed code block")),
            "{:?}",
            result.issues
        );
    }

    #[test]
    fn long_line_check_counts_characters() {
        let content = format!("# T\n{}\n", "é".repeat(300)); // 600 bytes, 300 chars
        let result = validate_claude(Path::new("x/SKILL.md"), &content);
        assert!(!result
            .issues
            .iter()
            .any(|i| i.message.contains("Very long line")));
    }

    #[test]
    fn skill_md_is_named_after_its_directory() {
        let result = validate_claude(Path::new("/s/my-skill/SKILL.md"), "# T\nBody");
        assert_eq!(result.name, "my-skill");
    }
}
