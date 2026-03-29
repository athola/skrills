//! Shared utilities for the Cursor adapter.
//!
//! Provides frontmatter parsing, stripping, and rendering for Cursor's
//! various file formats (agents with YAML frontmatter, skills without,
//! rules with `.mdc` frontmatter).

use regex::Regex;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Re-exports `split_frontmatter` from the shared adapter utilities.
///
/// This is a cross-adapter primitive for splitting YAML frontmatter delimiters.
/// Kept here for backwards compatibility with callers in the cursor adapter.
pub use crate::adapters::utils::split_frontmatter;

/// Parses YAML frontmatter from content, returning (frontmatter_fields, body).
///
/// Frontmatter is delimited by `---` on its own line at the start of the file.
/// Returns `(empty_map, full_content)` if no frontmatter is found.
pub fn parse_frontmatter(content: &str) -> (HashMap<String, String>, &str) {
    let (raw, body) = split_frontmatter(content);

    let Some(frontmatter_str) = raw else {
        return (HashMap::new(), body);
    };

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

/// Extracts the `model_hint` value from raw YAML frontmatter.
///
/// Returns the hint (e.g., "fast", "standard", "deep") or None if absent.
pub fn extract_model_hint(content: &str) -> Option<String> {
    let (raw, _) = split_frontmatter(content);
    raw.and_then(|fm| {
        fm.lines()
            .find(|line| line.trim().starts_with("model_hint:"))
            .and_then(|line| line.split_once(':'))
            .map(|(_, val)| val.trim().to_string())
    })
}

/// Section headings to strip during Cursor export.
///
/// Only strips sections that are purely navigational or meta-informational.
/// Sections with task-relevant content (Troubleshooting, Testing,
/// Verification, Technical Integration) are preserved to avoid
/// degrading model output quality.
const STRIP_HEADINGS: &[&str] = &["Supporting Modules", "See Also", "Table of Contents"];

/// Regex matching module reference links like `- [Name](modules/foo.md) - description`.
static MODULE_REF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^-\s*\[.*?\]\(modules/.*?\).*$\n?").expect("MODULE_REF regex should compile")
});

/// Regex matching 3+ consecutive blank lines.
static EXCESS_BLANKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\n{3,}").expect("EXCESS_BLANKS regex should compile"));

/// Trims non-essential sections from a skill body for Cursor export.
///
/// Strips: Troubleshooting, Supporting Modules, See Also, Testing,
/// Verification, Technical Integration, Table of Contents sections.
/// Also removes module file references and collapses excess blank lines.
///
/// This reduces token cost by ~40% on average across the skill catalog.
pub fn trim_skill_body(body: &str) -> String {
    // Strip sections by finding their headings and removing until the next heading
    let mut result = String::with_capacity(body.len());
    let mut skip = false;

    for line in body.lines() {
        // Check if this line is a ## heading
        if let Some(heading_text) = line.strip_prefix("## ") {
            let heading_trimmed = heading_text.trim();
            if STRIP_HEADINGS
                .iter()
                .any(|h| heading_trimmed.starts_with(h))
            {
                skip = true;
                continue;
            }
            // A different ## heading ends the skip
            skip = false;
        }

        if !skip {
            result.push_str(line);
            result.push('\n');
        }
    }

    let trimmed = MODULE_REF.replace_all(&result, "");
    let trimmed = EXCESS_BLANKS.replace_all(&trimmed, "\n\n");
    let trimmed = trimmed.trim_end_matches('\n');
    // Preserve a single trailing newline for POSIX compliance
    format!("{trimmed}\n")
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
    fn parse_frontmatter_crlf_line_endings() {
        let content =
            "---\r\nname: test-skill\r\ndescription: A test\r\n---\r\n\r\n# Body\r\n\r\nContent here.\r\n";
        let (fields, body) = parse_frontmatter(content);
        assert_eq!(fields.get("name").unwrap(), "test-skill");
        assert_eq!(fields.get("description").unwrap(), "A test");
        assert!(
            body.starts_with("# Body"),
            "Body should start with '# Body', got: {:?}",
            body
        );
    }

    #[test]
    fn split_frontmatter_crlf_line_endings() {
        let content = "---\r\nname: test\r\n---\r\n\r\n# Body\r\n";
        let (raw, body) = split_frontmatter(content);
        assert!(raw.is_some(), "Should find frontmatter with CRLF endings");
        let fm = raw.unwrap();
        assert!(
            fm.contains("name: test"),
            "Frontmatter should contain 'name: test', got: {:?}",
            fm
        );
        assert!(
            body.starts_with("# Body"),
            "Body should start with '# Body', got: {:?}",
            body
        );
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

    #[test]
    fn extract_model_hint_returns_value() {
        let content = "---\nname: test\nmodel_hint: fast\n---\n\n# Body\n";
        assert_eq!(extract_model_hint(content), Some("fast".to_string()));
    }

    #[test]
    fn extract_model_hint_missing_returns_none() {
        let content = "---\nname: test\n---\n\n# Body\n";
        assert_eq!(extract_model_hint(content), None);
    }

    #[test]
    fn extract_model_hint_no_frontmatter_returns_none() {
        let content = "# Just body\n";
        assert_eq!(extract_model_hint(content), None);
    }

    #[test]
    fn trim_skill_body_preserves_troubleshooting() {
        let body = "# Main\n\nContent here.\n\n## Troubleshooting\n\nFix stuff.\n";
        let result = trim_skill_body(body);
        assert!(
            result.contains("Troubleshooting"),
            "Troubleshooting has task-relevant content and should be preserved"
        );
        assert!(result.contains("Content here."));
    }

    #[test]
    fn trim_skill_body_strips_see_also() {
        let body = "# Main\n\nContent.\n\n## See Also\n\n- Link\n- Link2\n";
        let result = trim_skill_body(body);
        assert!(!result.contains("See Also"));
        assert!(result.contains("Content."));
    }

    #[test]
    fn trim_skill_body_strips_module_refs() {
        let body =
            "# Main\n\n- [Output templates](modules/output-templates.md) - formats\n\nOther.\n";
        let result = trim_skill_body(body);
        assert!(!result.contains("modules/"));
        assert!(result.contains("Other."));
    }

    #[test]
    fn trim_skill_body_preserves_essential_content() {
        let body = "# Steps\n\n1. Do this\n2. Do that\n\n## Rules\n\nNever do X.\n";
        let result = trim_skill_body(body);
        assert!(result.contains("Do this"));
        assert!(result.contains("Never do X."));
    }

    #[test]
    fn trim_skill_body_collapses_blank_lines() {
        let body = "Line 1\n\n\n\n\nLine 2\n";
        let result = trim_skill_body(body);
        assert!(!result.contains("\n\n\n"));
    }
}
