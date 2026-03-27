//! Auto-fix functionality for skills.
//!
//! Provides utilities to automatically fix frontmatter and body content
//! to make skills compatible with Codex CLI, Copilot CLI, and Cursor.

use crate::codex::{MAX_DESCRIPTION_LENGTH, MAX_NAME_LENGTH};
use crate::frontmatter::{generate_frontmatter, has_frontmatter, parse_frontmatter};
use std::fs;
use std::path::Path;

/// Minimum word count for a skill body to be considered complete.
const MIN_BODY_WORDS: usize = 30;

/// Result of an autofix operation.
#[derive(Debug, Clone)]
pub struct AutofixResult {
    /// Whether changes were made.
    pub modified: bool,
    /// The new content (if modified).
    pub content: String,
    /// Description of what was changed.
    pub changes: Vec<String>,
    /// Path to backup file (if created).
    pub backup_path: Option<std::path::PathBuf>,
}

/// Options for autofix behavior.
#[derive(Debug, Clone, Default)]
pub struct AutofixOptions {
    /// Create a backup before modifying.
    pub create_backup: bool,
    /// Actually write the changes to disk.
    pub write_changes: bool,
    /// Override name (if not present in content).
    pub suggested_name: Option<String>,
    /// Override description (if not present in content).
    pub suggested_description: Option<String>,
}

/// Generate a name from a file path.
pub fn derive_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| {
            // Convert SKILL to just the skill name, handle nested paths
            if s.eq_ignore_ascii_case("SKILL") {
                // Try to get parent directory name
                path.parent()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or("skill")
                    .to_string()
            } else {
                s.to_string()
            }
        })
        .unwrap_or_else(|| "skill".to_string())
}

/// Extract a description from markdown content.
///
/// Looks for the first paragraph or heading to use as a description.
pub fn derive_description_from_content(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();

    for line in lines.iter() {
        let trimmed = line.trim();

        // Skip empty lines and headings
        if trimmed.is_empty() {
            continue;
        }

        // If it's a heading, extract the text
        if trimmed.starts_with('#') {
            let text = trimmed.trim_start_matches('#').trim();
            if !text.is_empty() {
                return truncate_description(text);
            }
            continue;
        }

        // Skip code blocks
        if trimmed.starts_with("```") || trimmed.starts_with("    ") {
            continue;
        }

        // Use first non-empty, non-heading line as description
        return truncate_description(trimmed);
    }

    "A skill file".to_string()
}

fn truncate_description(desc: &str) -> String {
    if desc.len() <= MAX_DESCRIPTION_LENGTH {
        desc.to_string()
    } else {
        let mut truncated = desc[..MAX_DESCRIPTION_LENGTH - 3].to_string();
        truncated.push_str("...");
        truncated
    }
}

/// Normalize a name to kebab-case.
pub fn to_kebab_case(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Scaffold body sections when content is too short.
///
/// Adds Overview, Instructions, When to Use, and Output Format sections
/// based on the skill's description.
pub fn scaffold_body(body: &str, description: &str) -> (String, Vec<String>) {
    let words: usize = body.split_whitespace().count();
    if words >= MIN_BODY_WORDS {
        return (body.to_string(), vec![]);
    }

    let mut changes = Vec::new();
    let mut result = body.trim().to_string();

    let has_heading = body.lines().any(|l| l.starts_with('#'));
    let has_instructions = body.to_lowercase().contains("instructions")
        || body.to_lowercase().contains("steps")
        || body.to_lowercase().contains("workflow");
    let has_when = body.to_lowercase().contains("when to use")
        || body.to_lowercase().contains("triggers")
        || body.to_lowercase().contains("use when");
    let has_output = body.to_lowercase().contains("output")
        || body.to_lowercase().contains("format")
        || body.to_lowercase().contains("response");

    if !has_heading {
        result.push_str("\n\n## Overview\n\n");
        result.push_str(description);
        result.push('\n');
        changes.push("Added Overview section from description".to_string());
    }

    if !has_instructions {
        result.push_str("\n## Instructions\n\n");
        result.push_str("1. Analyze the context and requirements\n");
        result.push_str("2. Apply the skill logic\n");
        result.push_str("3. Validate the output\n");
        changes.push("Added Instructions section template".to_string());
    }

    let desc_lower = description.to_lowercase();
    if !has_when {
        result.push_str("\n## When to Use\n\n");
        result.push_str(&format!("Use this skill when you need to {desc_lower}.\n"));
        changes.push("Added When to Use section".to_string());
    }

    if !has_output {
        result.push_str("\n## Output Format\n\n");
        result.push_str("Provide clear, structured output appropriate to the task.\n");
        changes.push("Added Output Format section template".to_string());
    }

    (result, changes)
}

/// Add or update frontmatter to make a skill Codex-compatible.
pub fn autofix_frontmatter(
    path: &Path,
    content: &str,
    options: &AutofixOptions,
) -> Result<AutofixResult, String> {
    let mut changes = Vec::new();
    let mut modified = false;

    let new_content = if has_frontmatter(content) {
        // Parse existing frontmatter and potentially update it
        let parsed = parse_frontmatter(content)?;

        if let Some(fm) = parsed.frontmatter {
            // Track if fields were originally missing
            let name_was_missing = fm.name.is_none();
            let desc_was_missing = fm.description.is_none();

            let name = fm
                .name
                .or(options.suggested_name.clone())
                .unwrap_or_else(|| derive_name_from_path(path));

            let description = fm
                .description
                .or(options.suggested_description.clone())
                .unwrap_or_else(|| derive_description_from_content(&parsed.content));

            // Normalize name to kebab-case
            let kebab = to_kebab_case(&name);
            let name_normalized = kebab != name;
            let name = if name_normalized {
                changes.push(format!("Normalized name to kebab-case: {kebab}"));
                modified = true;
                kebab
            } else {
                name
            };

            // Truncate if needed
            let name = if name.chars().count() > MAX_NAME_LENGTH {
                changes.push(format!(
                    "Truncated name from {} to {} chars",
                    name.chars().count(),
                    MAX_NAME_LENGTH
                ));
                modified = true;
                name.chars().take(MAX_NAME_LENGTH).collect::<String>()
            } else {
                name
            };

            let description = if description.len() > MAX_DESCRIPTION_LENGTH {
                changes.push(format!(
                    "Truncated description from {} to {} chars",
                    description.len(),
                    MAX_DESCRIPTION_LENGTH
                ));
                modified = true;
                truncate_description(&description)
            } else {
                description
            };

            // Check if we need to add missing fields
            if name_was_missing {
                changes.push(format!("Added name: {name}"));
                modified = true;
            }
            if desc_was_missing {
                changes.push("Added description".to_string());
                modified = true;
            }

            // Scaffold body if too short
            let (body, body_changes) = scaffold_body(&parsed.content, &description);
            if !body_changes.is_empty() {
                changes.extend(body_changes);
                modified = true;
            }

            if modified {
                format!("{}{}", generate_frontmatter(&name, &description), body)
            } else {
                content.to_string()
            }
        } else {
            content.to_string()
        }
    } else {
        // No frontmatter - add it
        let name = to_kebab_case(
            &options
                .suggested_name
                .clone()
                .unwrap_or_else(|| derive_name_from_path(path)),
        );

        let description = options
            .suggested_description
            .clone()
            .unwrap_or_else(|| derive_description_from_content(content));

        changes.push(format!("Added frontmatter with name: {name}"));
        modified = true;

        // Scaffold body if too short
        let (body, body_changes) = scaffold_body(content, &description);
        changes.extend(body_changes);

        format!("{}\n{}", generate_frontmatter(&name, &description), body)
    };

    let mut result = AutofixResult {
        modified,
        content: new_content.clone(),
        changes,
        backup_path: None,
    };

    // Write changes if requested
    if modified && options.write_changes {
        // Create backup if requested
        if options.create_backup {
            let backup_path = path.with_extension("md.bak");
            fs::copy(path, &backup_path).map_err(|e| format!("Failed to create backup: {e}"))?;
            result.backup_path = Some(backup_path);
        }

        fs::write(path, &new_content).map_err(|e| format!("Failed to write changes: {e}"))?;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_derive_name_from_path() {
        assert_eq!(
            derive_name_from_path(Path::new("/skills/my-skill.md")),
            "my-skill"
        );
        assert_eq!(
            derive_name_from_path(Path::new("/skills/test-skill/SKILL.md")),
            "test-skill"
        );
    }

    #[test]
    fn test_derive_description_from_content() {
        let content = "# My Skill\n\nThis does something useful.";
        assert_eq!(derive_description_from_content(content), "My Skill");

        let content2 = "This is the first line.\n\n# Heading";
        assert_eq!(
            derive_description_from_content(content2),
            "This is the first line."
        );
    }

    #[test]
    fn test_autofix_no_frontmatter() {
        let content = "# My Skill\nSome content here.";
        let options = AutofixOptions::default();
        let result =
            autofix_frontmatter(Path::new("/test/my-skill.md"), content, &options).unwrap();

        assert!(result.modified);
        assert!(result.content.starts_with("---\n"));
        assert!(result.content.contains("name: my-skill"));
    }

    #[test]
    fn test_autofix_missing_name() {
        let content = "---\ndescription: Has description\n---\n# Content";
        let options = AutofixOptions::default();
        let result = autofix_frontmatter(Path::new("/test/skill.md"), content, &options).unwrap();

        assert!(result.modified);
        assert!(result.changes.iter().any(|c| c.contains("Added name")));
    }

    #[test]
    fn test_truncate_description() {
        let long = "a".repeat(600);
        let truncated = truncate_description(&long);
        assert_eq!(truncated.len(), MAX_DESCRIPTION_LENGTH);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn test_to_kebab_case() {
        assert_eq!(to_kebab_case("My Cool Skill"), "my-cool-skill");
        assert_eq!(to_kebab_case("already-kebab"), "already-kebab");
        assert_eq!(to_kebab_case("CamelCase"), "camelcase");
        assert_eq!(to_kebab_case("with  spaces"), "with-spaces");
        assert_eq!(to_kebab_case("--leading-trailing--"), "leading-trailing");
    }

    #[test]
    fn test_scaffold_body_short_content() {
        let body = "Some skill stuff.";
        let (scaffolded, changes) = scaffold_body(body, "do something useful");
        assert!(!changes.is_empty());
        assert!(scaffolded.contains("## Overview"));
        assert!(scaffolded.contains("## Instructions"));
        assert!(scaffolded.contains("## When to Use"));
        assert!(scaffolded.contains("## Output Format"));
        assert!(scaffolded.contains("do something useful"));
    }

    #[test]
    fn test_scaffold_body_already_complete() {
        let body = "This is a well-written skill with plenty of content. \
            It has many words already and does not need scaffolding. \
            The body is long enough to pass the minimum word count threshold easily.";
        let (scaffolded, changes) = scaffold_body(body, "test");
        assert!(changes.is_empty());
        assert_eq!(scaffolded, body);
    }

    #[test]
    fn test_scaffold_preserves_existing_sections() {
        let body = "## Instructions\n\n1. Do the thing\n2. Check the result";
        let (scaffolded, changes) = scaffold_body(body, "a skill");
        // Should add When to Use and Output Format but NOT Instructions or Overview (has heading)
        assert!(scaffolded.contains("## When to Use"));
        assert!(scaffolded.contains("## Output Format"));
        assert!(!changes.iter().any(|c| c.contains("Instructions")));
        assert!(!changes.iter().any(|c| c.contains("Overview")));
    }

    #[test]
    fn test_autofix_scaffolds_short_body() {
        let content = "---\nname: cool-skill\ndescription: does everything\n---\n\nShort body.";
        let options = AutofixOptions::default();
        let result =
            autofix_frontmatter(Path::new("/test/cool-skill.md"), content, &options).unwrap();
        assert!(result.modified);
        assert!(result.content.contains("## Instructions"));
        assert!(result.changes.iter().any(|c| c.contains("Instructions")));
    }

    #[test]
    fn test_truncate_name_multibyte_utf8() {
        // A name with multi-byte UTF-8 characters that exceeds MAX_NAME_LENGTH chars.
        // The old byte-index slice `name[..MAX_NAME_LENGTH]` would panic at a non-char
        // boundary; the fixed `chars().take()` must handle this safely.
        let prefix = "skill-";
        let multibyte_padding = "あ".repeat(MAX_NAME_LENGTH); // each あ is 3 bytes
        let long_name = format!("{prefix}{multibyte_padding}");
        assert!(long_name.chars().count() > MAX_NAME_LENGTH);

        let content = format!(
            "---\nname: {long_name}\ndescription: test multibyte\n---\n\n\
             Lots of content here to avoid scaffolding. This body has enough words \
             to pass the minimum threshold for body completeness checks."
        );
        let options = AutofixOptions::default();
        let result = autofix_frontmatter(Path::new("/test/skill.md"), &content, &options).unwrap();
        assert!(result.modified);
        assert!(result.changes.iter().any(|c| c.contains("Truncated name")));
        // Verify the truncated name is valid UTF-8 and within limits
        let fm_start = result.content.find("name: ").unwrap() + 6;
        let fm_end = result.content[fm_start..].find('\n').unwrap() + fm_start;
        let truncated_name = &result.content[fm_start..fm_end];
        assert!(truncated_name.chars().count() <= MAX_NAME_LENGTH);
        assert!(truncated_name.is_ascii() || truncated_name.contains('あ'));
    }

    #[test]
    fn test_autofix_normalizes_name_to_kebab() {
        let content = "---\nname: My Cool Skill\ndescription: a skill\n---\n\nLots of content here to avoid scaffolding. This body has enough words to pass the minimum threshold for body completeness checks.";
        let options = AutofixOptions::default();
        let result = autofix_frontmatter(Path::new("/test/skill.md"), content, &options).unwrap();
        assert!(result.modified);
        assert!(result.content.contains("name: my-cool-skill"));
        assert!(result.changes.iter().any(|c| c.contains("kebab-case")));
    }
}
