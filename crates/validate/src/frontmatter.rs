//! YAML frontmatter parsing for SKILL.md files.
//!
//! Codex CLI requires YAML frontmatter with:
//! - `name`: max 100 characters
//! - `description`: max 500 characters
//!
//! Claude Code is more permissive and doesn't require frontmatter.

use serde::{Deserialize, Serialize};

/// Parsed frontmatter from a SKILL.md file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    /// Skill name (max 100 chars for Codex).
    pub name: Option<String>,
    /// Skill description (max 500 chars for Codex).
    pub description: Option<String>,
    /// Additional fields that may be present.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_yaml::Value>,
}

/// Result of parsing frontmatter.
#[derive(Debug, Clone)]
pub struct ParsedSkill {
    /// Parsed frontmatter, if present.
    pub frontmatter: Option<SkillFrontmatter>,
    /// The markdown content after frontmatter.
    pub content: String,
    /// Raw frontmatter YAML string, if present.
    pub raw_frontmatter: Option<String>,
    /// Line number where content starts (1-indexed).
    pub content_start_line: usize,
}

/// Check if a skill file has frontmatter.
pub fn has_frontmatter(content: &str) -> bool {
    let trimmed = content.trim_start();
    trimmed.starts_with("---")
}

/// Split content into frontmatter and body sections.
///
/// Returns (frontmatter_yaml, body_content, content_start_line).
pub fn split_frontmatter(content: &str) -> (Option<String>, String, usize) {
    let trimmed = content.trim_start();

    if !trimmed.starts_with("---") {
        return (None, content.to_string(), 1);
    }

    // Count leading whitespace lines
    let leading_lines = content
        .lines()
        .take_while(|line| line.trim().is_empty())
        .count();

    // Find the closing ---
    let after_open = &trimmed[3..];
    let after_open = after_open.trim_start_matches(['\r', '\n']);

    if let Some(end_pos) = after_open.find("\n---") {
        let yaml = &after_open[..end_pos];
        let rest = &after_open[end_pos + 4..];
        let rest = rest.trim_start_matches(['\r', '\n']);

        // Calculate content start line
        let frontmatter_lines = yaml.lines().count() + 2; // +2 for opening and closing ---
        let content_start = leading_lines + frontmatter_lines + 1;

        (Some(yaml.to_string()), rest.to_string(), content_start)
    } else if let Some(end_pos) = after_open.find("\r\n---") {
        let yaml = &after_open[..end_pos];
        let rest = &after_open[end_pos + 5..];
        let rest = rest.trim_start_matches(['\r', '\n']);

        let frontmatter_lines = yaml.lines().count() + 2;
        let content_start = leading_lines + frontmatter_lines + 1;

        (Some(yaml.to_string()), rest.to_string(), content_start)
    } else {
        // No closing ---, treat entire content as body
        (None, content.to_string(), 1)
    }
}

/// Parse YAML frontmatter from a skill file.
pub fn parse_frontmatter(content: &str) -> Result<ParsedSkill, String> {
    let (raw_yaml, body, content_start) = split_frontmatter(content);

    let frontmatter = if let Some(ref yaml) = raw_yaml {
        match serde_yaml::from_str::<SkillFrontmatter>(yaml) {
            Ok(fm) => Some(fm),
            Err(e) => return Err(format!("Invalid YAML frontmatter: {e}")),
        }
    } else {
        None
    };

    Ok(ParsedSkill {
        frontmatter,
        content: body,
        raw_frontmatter: raw_yaml,
        content_start_line: content_start,
    })
}

/// Generate frontmatter YAML from a name and description.
pub fn generate_frontmatter(name: &str, description: &str) -> String {
    // Escape special characters in YAML strings
    let name_escaped = if name.contains(':') || name.contains('#') {
        format!("\"{}\"", name.replace('"', "\\\""))
    } else {
        name.to_string()
    };

    let desc_escaped = if description.contains('\n') || description.len() > 80 {
        // Use YAML literal block scalar for multi-line or long descriptions
        let indented = description
            .lines()
            .map(|line| format!("  {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!("|\n{indented}")
    } else if description.contains(':') || description.contains('#') {
        format!("\"{}\"", description.replace('"', "\\\""))
    } else {
        description.to_string()
    };

    format!("---\nname: {name_escaped}\ndescription: {desc_escaped}\n---\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_has_frontmatter() {
        assert!(has_frontmatter("---\nname: test\n---\n# Content"));
        assert!(has_frontmatter("  ---\nname: test\n---\n# Content"));
        assert!(!has_frontmatter("# No frontmatter"));
        assert!(!has_frontmatter("-- not quite"));
    }

    #[test]
    fn test_split_frontmatter() {
        let content = "---\nname: test\ndescription: A test skill\n---\n# Heading\nBody";
        let (yaml, body, line) = split_frontmatter(content);

        assert!(yaml.is_some());
        assert!(yaml.unwrap().contains("name: test"));
        assert!(body.starts_with("# Heading"));
        assert_eq!(line, 5);
    }

    #[test]
    fn test_split_no_frontmatter() {
        let content = "# Just markdown\nNo frontmatter here.";
        let (yaml, body, line) = split_frontmatter(content);

        assert!(yaml.is_none());
        assert_eq!(body, content);
        assert_eq!(line, 1);
    }

    #[test]
    fn test_parse_frontmatter() {
        let content = "---\nname: my-skill\ndescription: Does something useful\n---\n# My Skill";
        let parsed = parse_frontmatter(content).unwrap();

        assert!(parsed.frontmatter.is_some());
        let fm = parsed.frontmatter.unwrap();
        assert_eq!(fm.name, Some("my-skill".to_string()));
        assert_eq!(fm.description, Some("Does something useful".to_string()));
        assert!(parsed.content.starts_with("# My Skill"));
    }

    #[test]
    fn test_generate_frontmatter() {
        let fm = generate_frontmatter("test-skill", "A simple test skill");
        assert!(fm.contains("name: test-skill"));
        assert!(fm.contains("description: A simple test skill"));
        assert!(fm.starts_with("---\n"));
        assert!(fm.ends_with("---\n"));
    }

    #[test]
    fn test_generate_frontmatter_special_chars() {
        let fm = generate_frontmatter("skill:name", "Contains: colon");
        assert!(fm.contains("name: \"skill:name\""));
        assert!(fm.contains("description: \"Contains: colon\""));
    }
}
