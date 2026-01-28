//! Utility functions for Copilot adapter.

/// Sanitizes a skill name to prevent path traversal attacks.
///
/// # Security Rationale
///
/// This function is **critical for security** because skill names are used to construct
/// file paths. Without sanitization, a malicious skill name like `../../../etc/passwd`
/// could escape the intended directory and read/write arbitrary files on the filesystem.
///
/// The sanitization prevents:
/// - **Directory traversal**: `..` segments that could escape the skill directory
/// - **Absolute paths**: Would be split and lose the leading `/`
/// - **Hidden files**: Dots at segment start are preserved but traversal is blocked
///
/// # Behavior
///
/// Preserves forward slashes for nested skill directories (e.g., `category/my-skill`)
/// while preventing path traversal attacks (e.g., `../../../etc/passwd`).
///
/// Each path segment is sanitized to only allow alphanumeric characters, hyphens,
/// and underscores. Empty segments and `.` or `..` are removed.
pub fn sanitize_name(name: &str) -> String {
    name.split('/')
        .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        .map(|segment| {
            segment
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Transforms a Claude agent's content to Copilot agent format.
///
/// Transformations:
/// - Replaces `model: xxx` with `target: github-copilot`
/// - Removes `color: xxx` line (Copilot doesn't use this)
/// - Keeps everything else intact
pub fn transform_agent_for_copilot(content: &[u8]) -> Vec<u8> {
    let content_str = match std::str::from_utf8(content) {
        Ok(s) => s,
        Err(_) => return content.to_vec(), // Binary content, return as-is
    };

    // Check if content has YAML frontmatter
    if !content_str.starts_with("---") {
        // No frontmatter, add minimal frontmatter with target
        return format!("---\ntarget: github-copilot\n---\n\n{}", content_str).into_bytes();
    }

    // Find the end of frontmatter
    let Some(end_idx) = content_str[3..].find("\n---").map(|i| i + 3) else {
        // Malformed frontmatter, return as-is
        return content.to_vec();
    };

    let frontmatter = &content_str[3..end_idx];
    let body = &content_str[end_idx + 4..]; // Skip "\n---"

    let mut new_lines = Vec::new();
    let mut has_target = false;

    for line in frontmatter.lines() {
        let trimmed = line.trim();

        // Skip model and color lines (Claude-specific)
        if trimmed.starts_with("model:") || trimmed.starts_with("color:") {
            continue;
        }

        // Check if target already exists
        if trimmed.starts_with("target:") {
            has_target = true;
        }

        new_lines.push(line);
    }

    // Add target if not already present
    if !has_target {
        new_lines.push("target: github-copilot");
    }

    format!("---\n{}\n---{}", new_lines.join("\n"), body).into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_path_traversal_attack() {
        assert_eq!(sanitize_name("../../../etc/passwd"), "etc/passwd");
    }

    #[test]
    fn test_sanitize_double_dot_alone() {
        assert_eq!(sanitize_name(".."), "");
    }

    #[test]
    fn test_sanitize_single_dot_alone() {
        assert_eq!(sanitize_name("."), "");
    }

    #[test]
    fn test_sanitize_strips_dotdot_segment() {
        assert_eq!(sanitize_name("foo/../bar"), "foo/bar");
    }

    #[test]
    fn test_sanitize_strips_dot_segment() {
        assert_eq!(sanitize_name("foo/./bar"), "foo/bar");
    }

    #[test]
    fn test_sanitize_normal_name() {
        assert_eq!(sanitize_name("my-skill"), "my-skill");
    }

    #[test]
    fn test_sanitize_nested_valid_name() {
        assert_eq!(sanitize_name("category/my-skill"), "category/my-skill");
    }

    #[test]
    fn test_sanitize_empty_string() {
        assert_eq!(sanitize_name(""), "");
    }

    #[test]
    fn test_sanitize_special_chars() {
        assert_eq!(sanitize_name("my skill!@#"), "myskill");
    }

    #[test]
    fn test_sanitize_multiple_slashes() {
        assert_eq!(sanitize_name("foo///bar"), "foo/bar");
    }

    #[test]
    fn test_transform_no_frontmatter() {
        let content = b"This is agent content";
        let result = transform_agent_for_copilot(content);
        let result_str = std::str::from_utf8(&result).unwrap();

        assert!(result_str.starts_with("---\ntarget: github-copilot\n---\n\n"));
        assert!(result_str.contains("This is agent content"));
    }

    #[test]
    fn test_transform_with_model_line() {
        let content = b"---\nmodel: claude-opus-4\nname: test\n---\n\nContent here";
        let result = transform_agent_for_copilot(content);
        let result_str = std::str::from_utf8(&result).unwrap();

        assert!(result_str.contains("target: github-copilot"));
        assert!(!result_str.contains("model:"));
        assert!(result_str.contains("name: test"));
        assert!(result_str.contains("Content here"));
    }

    #[test]
    fn test_transform_with_existing_target() {
        let content = b"---\ntarget: existing-target\nname: test\n---\n\nContent here";
        let result = transform_agent_for_copilot(content);
        let result_str = std::str::from_utf8(&result).unwrap();

        assert!(result_str.contains("target: existing-target"));
        assert_eq!(result_str.matches("target:").count(), 1);
    }
}
