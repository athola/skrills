//! Auto-fix functionality for skills.
//!
//! Provides utilities to automatically add or fix frontmatter
//! to make skills compatible with Codex CLI.

use crate::codex::{MAX_DESCRIPTION_LENGTH, MAX_NAME_LENGTH};
use crate::frontmatter::{generate_frontmatter, has_frontmatter, parse_frontmatter};
use std::fs;
use std::path::Path;

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

            // Truncate if needed
            let name = if name.len() > MAX_NAME_LENGTH {
                changes.push(format!(
                    "Truncated name from {} to {} chars",
                    name.len(),
                    MAX_NAME_LENGTH
                ));
                modified = true;
                name[..MAX_NAME_LENGTH].to_string()
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

            if modified {
                format!(
                    "{}{}",
                    generate_frontmatter(&name, &description),
                    parsed.content
                )
            } else {
                content.to_string()
            }
        } else {
            content.to_string()
        }
    } else {
        // No frontmatter - add it
        let name = options
            .suggested_name
            .clone()
            .unwrap_or_else(|| derive_name_from_path(path));

        let description = options
            .suggested_description
            .clone()
            .unwrap_or_else(|| derive_description_from_content(content));

        changes.push(format!("Added frontmatter with name: {name}"));
        modified = true;

        format!("{}\n{}", generate_frontmatter(&name, &description), content)
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
}
