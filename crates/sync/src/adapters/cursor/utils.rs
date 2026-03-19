//! Shared utilities for the Cursor adapter.
//!
//! Provides frontmatter parsing, stripping, and rendering for Cursor's
//! various file formats (agents with YAML frontmatter, skills without,
//! rules with `.mdc` frontmatter).

use std::collections::HashMap;

/// Parses YAML frontmatter from content, returning (frontmatter_fields, body).
///
/// Frontmatter is delimited by `---` on its own line at the start of the file.
/// Returns `(empty_map, full_content)` if no frontmatter is found.
pub fn parse_frontmatter(content: &str) -> (HashMap<String, String>, &str) {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return (HashMap::new(), content);
    }

    // Find the closing `---`
    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);

    if let Some(close_pos) = after_open.find("\n---") {
        let frontmatter_str = &after_open[..close_pos];
        let body_start = close_pos + 4; // skip "\n---"
        let body = &after_open[body_start..];
        let body = body.trim_start_matches(['\n', '\r']);

        // Parse simple key: value pairs from frontmatter
        let mut fields = HashMap::new();
        for line in frontmatter_str.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, rest)) = line.split_once(':') {
                let key = key.trim().to_string();
                let value = rest.trim().to_string();
                fields.insert(key, value);
            }
        }

        (fields, body)
    } else {
        // No closing delimiter — treat entire content as body
        (HashMap::new(), content)
    }
}

/// Strips YAML frontmatter from content, returning only the body.
///
/// Used when writing Claude skills to Cursor (Cursor skills have no frontmatter).
pub fn strip_frontmatter(content: &str) -> &str {
    let (_fields, body) = parse_frontmatter(content);
    body
}

/// Renders YAML frontmatter fields + body into a complete document.
///
/// Fields are written in sorted order for deterministic output.
pub fn render_frontmatter(fields: &HashMap<String, String>, body: &str) -> String {
    if fields.is_empty() {
        return body.to_string();
    }

    let mut result = String::from("---\n");
    let mut sorted_keys: Vec<&String> = fields.keys().collect();
    sorted_keys.sort();

    for key in sorted_keys {
        result.push_str(&format!("{}: {}\n", key, fields[key]));
    }
    result.push_str("---\n");

    if !body.is_empty() {
        // Ensure single newline between frontmatter and body
        if !body.starts_with('\n') {
            result.push('\n');
        }
        result.push_str(body);
    }

    result
}

/// Sanitizes a name to kebab-case suitable for Cursor file/directory names.
///
/// Re-exports the shared `sanitize_name_kebab` from `adapters::utils`.
pub fn sanitize_name(name: &str) -> String {
    crate::adapters::utils::sanitize_name_kebab(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_frontmatter_with_fields() {
        let content =
            "---\nname: test-skill\ndescription: A test\n---\n\n# Body\n\nContent here.\n";
        let (fields, body) = parse_frontmatter(content);
        assert_eq!(fields.get("name").unwrap(), "test-skill");
        assert_eq!(fields.get("description").unwrap(), "A test");
        assert!(body.starts_with("# Body"));
    }

    #[test]
    fn parse_frontmatter_without_frontmatter() {
        let content = "# Just a markdown file\n\nNo frontmatter.\n";
        let (fields, body) = parse_frontmatter(content);
        assert!(fields.is_empty());
        assert_eq!(body, content);
    }

    #[test]
    fn strip_frontmatter_removes_yaml() {
        let content = "---\nname: test\ntags: [a, b]\n---\n\n# Title\n\nBody.\n";
        let body = strip_frontmatter(content);
        assert!(body.starts_with("# Title"));
        assert!(!body.contains("---"));
    }

    #[test]
    fn strip_frontmatter_no_frontmatter() {
        let content = "# Just content\n";
        assert_eq!(strip_frontmatter(content), content);
    }

    #[test]
    fn render_frontmatter_produces_valid_yaml() {
        let mut fields = HashMap::new();
        fields.insert("alwaysApply".to_string(), "true".to_string());
        fields.insert("description".to_string(), "My rule".to_string());
        let body = "# Rule content\n\nDo the thing.\n";

        let result = render_frontmatter(&fields, body);
        assert!(result.starts_with("---\n"));
        assert!(result.contains("alwaysApply: true\n"));
        assert!(result.contains("description: My rule\n"));
        assert!(result.contains("# Rule content"));
    }

    #[test]
    fn render_frontmatter_empty_fields() {
        let fields = HashMap::new();
        let body = "# Just body\n";
        assert_eq!(render_frontmatter(&fields, body), body);
    }

    #[test]
    fn sanitize_name_converts_to_kebab() {
        assert_eq!(sanitize_name("My Skill Name"), "my-skill-name");
        assert_eq!(
            sanitize_name("skill_with_underscores"),
            "skill-with-underscores"
        );
        assert_eq!(sanitize_name("Already-Kebab"), "already-kebab");
        assert_eq!(sanitize_name("file.name.ext"), "file-name-ext");
    }
}
