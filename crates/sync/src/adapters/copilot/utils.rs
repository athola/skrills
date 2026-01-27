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
