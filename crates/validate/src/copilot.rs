//! GitHub Copilot CLI skill validation.
//!
//! Copilot CLI has strict requirements similar to Codex:
//! - YAML frontmatter is required
//! - `name` field required (max 100 characters)
//! - `description` field required (max 500 characters)
//! - Content should be under 30,000 characters (warning)

use crate::common::{ValidationIssue, ValidationResult, ValidationTarget};
use crate::frontmatter::{parse_frontmatter, ParsedSkill};
use std::path::Path;

/// Maximum length for skill name in Copilot.
pub const MAX_NAME_LENGTH: usize = 100;

/// Maximum length for skill description in Copilot.
pub const MAX_DESCRIPTION_LENGTH: usize = 500;

/// Maximum recommended content length for Copilot skills.
///
/// # Rationale
///
/// The 30,000 character limit is based on practical token budget constraints:
/// - Copilot's context window must accommodate the skill content plus user prompts
/// - Skills exceeding this limit consume disproportionate context, leaving less
///   room for conversation history and code context
/// - Very large skills often indicate they should be split into focused sub-skills
///
/// This is a **warning** threshold, not a hard limit. Skills can exceed this but
/// will trigger a recommendation to consider splitting.
pub const MAX_CONTENT_LENGTH: usize = 30_000;

/// Validate a skill for Copilot CLI compatibility.
pub fn validate_copilot(path: &Path, content: &str) -> ValidationResult {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut result = ValidationResult::new(path.to_path_buf(), name.clone());

    // Parse frontmatter (required for Copilot)
    let parsed = match parse_frontmatter(content) {
        Ok(p) => p,
        Err(e) => {
            result.add_issue(
                ValidationIssue::error(
                    ValidationTarget::Copilot,
                    format!("Invalid YAML frontmatter: {e}"),
                )
                .with_line(1),
            );
            return result;
        }
    };

    // Frontmatter is required for Copilot
    if parsed.frontmatter.is_none() {
        result.add_issue(
            ValidationIssue::error(
                ValidationTarget::Copilot,
                "Copilot CLI requires YAML frontmatter with name and description",
            )
            .with_suggestion(
                "Add frontmatter: ---\\nname: skill-name\\ndescription: Description\\n---",
            ),
        );
        return result;
    }

    // SAFETY: We just checked `parsed.frontmatter.is_none()` above and returned early,
    // so this unwrap is guaranteed to succeed. Using `let Some` pattern for defense in depth.
    let Some(fm) = parsed.frontmatter.as_ref() else {
        // This branch should never be reached due to the early return above,
        // but we handle it defensively to prevent potential panics if code is refactored.
        return result;
    };

    // Update name from frontmatter
    if let Some(ref n) = fm.name {
        result.name = n.clone();
    }

    // Validate required fields
    validate_copilot_frontmatter(&mut result, fm);

    // Validate content
    validate_copilot_content(&mut result, &parsed);

    result
}

fn validate_copilot_frontmatter(
    result: &mut ValidationResult,
    fm: &crate::frontmatter::SkillFrontmatter,
) {
    // Name is required
    match &fm.name {
        None => {
            result.add_issue(
                ValidationIssue::error(ValidationTarget::Copilot, "Missing required 'name' field")
                    .with_line(2)
                    .with_suggestion("Add 'name: your-skill-name' to frontmatter"),
            );
        }
        Some(name) => {
            if name.is_empty() {
                result.add_issue(
                    ValidationIssue::error(
                        ValidationTarget::Copilot,
                        "'name' field cannot be empty",
                    )
                    .with_line(2),
                );
            } else if name.len() > MAX_NAME_LENGTH {
                result.add_issue(
                    ValidationIssue::error(
                        ValidationTarget::Copilot,
                        format!(
                            "'name' exceeds maximum length ({} > {} chars)",
                            name.len(),
                            MAX_NAME_LENGTH
                        ),
                    )
                    .with_line(2),
                );
            }

            // Check for invalid characters in name
            if name.contains('\n') || name.contains('\r') {
                result.add_issue(
                    ValidationIssue::error(
                        ValidationTarget::Copilot,
                        "'name' cannot contain newlines",
                    )
                    .with_line(2),
                );
            }
        }
    }

    // Description is required
    match &fm.description {
        None => {
            result.add_issue(
                ValidationIssue::error(
                    ValidationTarget::Copilot,
                    "Missing required 'description' field",
                )
                .with_line(3)
                .with_suggestion(
                    "Add 'description: Brief description of what the skill does' to frontmatter",
                ),
            );
        }
        Some(desc) => {
            if desc.is_empty() {
                result.add_issue(
                    ValidationIssue::error(
                        ValidationTarget::Copilot,
                        "'description' field cannot be empty",
                    )
                    .with_line(3),
                );
            } else if desc.len() > MAX_DESCRIPTION_LENGTH {
                result.add_issue(
                    ValidationIssue::error(
                        ValidationTarget::Copilot,
                        format!(
                            "'description' exceeds maximum length ({} > {} chars)",
                            desc.len(),
                            MAX_DESCRIPTION_LENGTH
                        ),
                    )
                    .with_line(3)
                    .with_suggestion("Shorten the description to 500 characters or less"),
                );
            }
        }
    }
}

fn validate_copilot_content(result: &mut ValidationResult, parsed: &ParsedSkill) {
    // Check for empty content
    if parsed.content.trim().is_empty() {
        result.add_issue(ValidationIssue::error(
            ValidationTarget::Copilot,
            "Skill has no content after frontmatter",
        ));
        return;
    }

    // Copilot-specific: warn if content exceeds 30,000 characters
    let content_len = parsed.content.len();
    if content_len > MAX_CONTENT_LENGTH {
        result.add_issue(
            ValidationIssue::warning(
                ValidationTarget::Copilot,
                format!(
                    "Content exceeds recommended limit ({} > {} chars)",
                    content_len, MAX_CONTENT_LENGTH
                ),
            )
            .with_suggestion("Consider breaking this skill into smaller, focused skills"),
        );
    }
}

/// Check if a skill is Copilot-compatible.
pub fn is_copilot_compatible(content: &str) -> bool {
    let parsed = match parse_frontmatter(content) {
        Ok(p) => p,
        Err(_) => return false,
    };

    if let Some(fm) = parsed.frontmatter {
        // Must have both name and description within limits
        let name_ok = fm
            .name
            .as_ref()
            .map(|n| !n.is_empty() && n.len() <= MAX_NAME_LENGTH)
            .unwrap_or(false);

        let desc_ok = fm
            .description
            .as_ref()
            .map(|d| !d.is_empty() && d.len() <= MAX_DESCRIPTION_LENGTH)
            .unwrap_or(false);

        name_ok && desc_ok && !parsed.content.trim().is_empty()
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ==========================================
    // Valid Skill Tests
    // ==========================================

    #[test]
    fn given_valid_skill_when_validated_then_copilot_valid() {
        let content =
            "---\nname: test-skill\ndescription: A test skill for validation\n---\n# Content\nBody here.";
        let result = validate_copilot(&PathBuf::from("test.md"), content);

        assert!(result.copilot_valid);
        assert_eq!(result.name, "test-skill");
        assert_eq!(result.error_count(), 0);
    }

    // ==========================================
    // Missing Frontmatter Tests
    // ==========================================

    #[test]
    fn given_missing_frontmatter_when_validated_then_copilot_invalid() {
        let content = "# No frontmatter\nJust content.";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("requires YAML frontmatter")));
    }

    #[test]
    fn given_missing_name_when_validated_then_copilot_invalid() {
        let content = "---\ndescription: Has description only\n---\n# Content";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("Missing required 'name'")));
    }

    #[test]
    fn given_missing_description_when_validated_then_copilot_invalid() {
        let content = "---\nname: has-name-only\n---\n# Content";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("Missing required 'description'")));
    }

    // ==========================================
    // Field Length Tests
    // ==========================================

    #[test]
    fn given_empty_name_when_validated_then_copilot_invalid() {
        let content = "---\nname: \"\"\ndescription: Test\n---\n# Content";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("cannot be empty")));
    }

    #[test]
    fn given_empty_description_when_validated_then_copilot_invalid() {
        let content = "---\nname: test\ndescription: \"\"\n---\n# Content";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("cannot be empty")));
    }

    #[test]
    fn given_name_with_newlines_when_validated_then_copilot_invalid() {
        let content = "---\nname: |\n  multi\n  line\ndescription: Test\n---\n# Content";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("cannot contain newlines")));
    }

    #[test]
    fn given_name_too_long_when_validated_then_copilot_invalid() {
        let long_name = "a".repeat(101);
        let content = format!("---\nname: {long_name}\ndescription: Test\n---\n# Content");
        let result = validate_copilot(&PathBuf::from("skill.md"), &content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("exceeds maximum length")));
    }

    #[test]
    fn given_description_too_long_when_validated_then_copilot_invalid() {
        let long_desc = "a".repeat(501);
        let content = format!("---\nname: test\ndescription: {long_desc}\n---\n# Content");
        let result = validate_copilot(&PathBuf::from("skill.md"), &content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("exceeds maximum length")));
    }

    // ==========================================
    // Content Length Tests (Copilot-specific)
    // ==========================================

    #[test]
    fn given_content_exceeds_30000_chars_when_validated_then_warning_issued() {
        let long_content = "x".repeat(30_001);
        let content = format!(
            "---\nname: test\ndescription: Test skill\n---\n# Content\n{}",
            long_content
        );
        let result = validate_copilot(&PathBuf::from("skill.md"), &content);

        // Should still be valid (warning, not error)
        assert!(result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("exceeds recommended limit")));
    }

    #[test]
    fn given_content_under_30000_chars_when_validated_then_no_warning() {
        let content = "---\nname: test\ndescription: Test skill\n---\n# Content\nNormal content.";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(result.copilot_valid);
        assert!(!result
            .issues
            .iter()
            .any(|i| i.message.contains("exceeds recommended limit")));
    }

    // ==========================================
    // Boundary Condition Tests
    // ==========================================

    #[test]
    fn given_name_at_max_length_when_validated_then_copilot_valid() {
        let name = "a".repeat(100); // Exactly MAX_NAME_LENGTH
        let content = format!("---\nname: {name}\ndescription: Test\n---\n# Content");
        let result = validate_copilot(&PathBuf::from("skill.md"), &content);

        assert!(result.copilot_valid);
    }

    #[test]
    fn given_description_at_max_length_when_validated_then_copilot_valid() {
        let desc = "a".repeat(500); // Exactly MAX_DESCRIPTION_LENGTH
        let content = format!("---\nname: test\ndescription: {desc}\n---\n# Content");
        let result = validate_copilot(&PathBuf::from("skill.md"), &content);

        assert!(result.copilot_valid);
    }

    // ==========================================
    // Empty Content Tests
    // ==========================================

    #[test]
    fn given_empty_content_when_validated_then_copilot_invalid() {
        let content = "---\nname: test\ndescription: Test\n---\n";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("no content after frontmatter")));
    }

    #[test]
    fn given_whitespace_only_content_when_validated_then_copilot_invalid() {
        let content = "---\nname: test\ndescription: Test\n---\n   \n\t\n  ";
        let result = validate_copilot(&PathBuf::from("skill.md"), content);

        assert!(!result.copilot_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("no content after frontmatter")));
    }

    // ==========================================
    // is_copilot_compatible Tests
    // ==========================================

    #[test]
    fn given_valid_skill_when_checking_compatibility_then_true() {
        assert!(is_copilot_compatible(
            "---\nname: test\ndescription: desc\n---\n# Content"
        ));
    }

    #[test]
    fn given_no_frontmatter_when_checking_compatibility_then_false() {
        assert!(!is_copilot_compatible("# No frontmatter"));
    }

    #[test]
    fn given_missing_description_when_checking_compatibility_then_false() {
        assert!(!is_copilot_compatible(
            "---\nname: test\n---\n# Missing desc"
        ));
    }

    #[test]
    fn given_missing_name_when_checking_compatibility_then_false() {
        assert!(!is_copilot_compatible(
            "---\ndescription: desc\n---\n# Missing name"
        ));
    }
}
