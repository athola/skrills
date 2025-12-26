//! Codex CLI skill validation.
//!
//! Codex CLI has strict requirements:
//! - YAML frontmatter is required
//! - `name` field required (max 100 characters)
//! - `description` field required (max 500 characters)

use crate::common::{ValidationIssue, ValidationResult, ValidationTarget};
use crate::frontmatter::{parse_frontmatter, ParsedSkill};
use std::path::Path;

/// Maximum length for skill name in Codex.
pub const MAX_NAME_LENGTH: usize = 100;

/// Maximum length for skill description in Codex.
pub const MAX_DESCRIPTION_LENGTH: usize = 500;

/// Validate a skill for Codex CLI compatibility.
pub fn validate_codex(path: &Path, content: &str) -> ValidationResult {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut result = ValidationResult::new(path.to_path_buf(), name.clone());

    // Parse frontmatter (required for Codex)
    let parsed = match parse_frontmatter(content) {
        Ok(p) => p,
        Err(e) => {
            result.add_issue(
                ValidationIssue::error(
                    ValidationTarget::Codex,
                    format!("Invalid YAML frontmatter: {e}"),
                )
                .with_line(1),
            );
            return result;
        }
    };

    // Frontmatter is required for Codex
    if parsed.frontmatter.is_none() {
        result.add_issue(
            ValidationIssue::error(
                ValidationTarget::Codex,
                "Codex CLI requires YAML frontmatter with name and description",
            )
            .with_suggestion(
                "Add frontmatter: ---\\nname: skill-name\\ndescription: Description\\n---",
            ),
        );
        return result;
    }

    let fm = parsed
        .frontmatter
        .as_ref()
        .expect("frontmatter required for Codex validation");

    // Update name from frontmatter
    if let Some(ref n) = fm.name {
        result.name = n.clone();
    }

    // Validate required fields
    validate_codex_frontmatter(&mut result, fm);

    // Validate content
    validate_codex_content(&mut result, &parsed);

    result
}

fn validate_codex_frontmatter(
    result: &mut ValidationResult,
    fm: &crate::frontmatter::SkillFrontmatter,
) {
    // Name is required
    match &fm.name {
        None => {
            result.add_issue(
                ValidationIssue::error(ValidationTarget::Codex, "Missing required 'name' field")
                    .with_line(2)
                    .with_suggestion("Add 'name: your-skill-name' to frontmatter"),
            );
        }
        Some(name) => {
            if name.is_empty() {
                result.add_issue(
                    ValidationIssue::error(ValidationTarget::Codex, "'name' field cannot be empty")
                        .with_line(2),
                );
            } else if name.len() > MAX_NAME_LENGTH {
                result.add_issue(
                    ValidationIssue::error(
                        ValidationTarget::Codex,
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
                        ValidationTarget::Codex,
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
                    ValidationTarget::Codex,
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
                        ValidationTarget::Codex,
                        "'description' field cannot be empty",
                    )
                    .with_line(3),
                );
            } else if desc.len() > MAX_DESCRIPTION_LENGTH {
                result.add_issue(
                    ValidationIssue::error(
                        ValidationTarget::Codex,
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

fn validate_codex_content(result: &mut ValidationResult, parsed: &ParsedSkill) {
    // Check for empty content
    if parsed.content.trim().is_empty() {
        result.add_issue(ValidationIssue::error(
            ValidationTarget::Codex,
            "Skill has no content after frontmatter",
        ));
    }
}

/// Check if a skill is Codex-compatible.
pub fn is_codex_compatible(content: &str) -> bool {
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

    #[test]
    fn test_valid_codex_skill() {
        let content = "---\nname: test-skill\ndescription: A test skill for validation\n---\n# Content\nBody here.";
        let result = validate_codex(&PathBuf::from("test.md"), content);

        assert!(result.codex_valid);
        assert_eq!(result.name, "test-skill");
        assert_eq!(result.error_count(), 0);
    }

    #[test]
    fn test_missing_frontmatter() {
        let content = "# No frontmatter\nJust content.";
        let result = validate_codex(&PathBuf::from("skill.md"), content);

        assert!(!result.codex_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("requires YAML frontmatter")));
    }

    #[test]
    fn test_missing_name() {
        let content = "---\ndescription: Has description only\n---\n# Content";
        let result = validate_codex(&PathBuf::from("skill.md"), content);

        assert!(!result.codex_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("Missing required 'name'")));
    }

    #[test]
    fn test_missing_description() {
        let content = "---\nname: has-name-only\n---\n# Content";
        let result = validate_codex(&PathBuf::from("skill.md"), content);

        assert!(!result.codex_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("Missing required 'description'")));
    }

    #[test]
    fn test_name_too_long() {
        let long_name = "a".repeat(101);
        let content = format!("---\nname: {long_name}\ndescription: Test\n---\n# Content");
        let result = validate_codex(&PathBuf::from("skill.md"), &content);

        assert!(!result.codex_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("exceeds maximum length")));
    }

    #[test]
    fn test_description_too_long() {
        let long_desc = "a".repeat(501);
        let content = format!("---\nname: test\ndescription: {long_desc}\n---\n# Content");
        let result = validate_codex(&PathBuf::from("skill.md"), &content);

        assert!(!result.codex_valid);
        assert!(result
            .issues
            .iter()
            .any(|i| i.message.contains("exceeds maximum length")));
    }

    #[test]
    fn test_is_codex_compatible() {
        assert!(is_codex_compatible(
            "---\nname: test\ndescription: desc\n---\n# Content"
        ));
        assert!(!is_codex_compatible("# No frontmatter"));
        assert!(!is_codex_compatible("---\nname: test\n---\n# Missing desc"));
        assert!(!is_codex_compatible(
            "---\ndescription: desc\n---\n# Missing name"
        ));
    }
}
