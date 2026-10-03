//! Utility functions for Copilot adapter.

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

    // Frontmatter only when the file opens with a line that is exactly `---`
    // and a later line is exactly `---`. A `----` rule or `---text` used to be
    // taken for frontmatter, and a closing line merely starting with `---`
    // leaked its remainder into the body.
    let Some((frontmatter, body)) = split_strict_frontmatter(content_str) else {
        if content_str.starts_with("---")
            && content_str.lines().next().map(str::trim_end) == Some("---")
        {
            // Opened but never closed: malformed, return as-is.
            return content.to_vec();
        }
        // No frontmatter, add minimal frontmatter with target
        return format!("---\ntarget: github-copilot\n---\n\n{}", content_str).into_bytes();
    };

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

    format!("---\n{}\n---\n{}", new_lines.join("\n"), body).into_bytes()
}

/// Splits `---` delimited frontmatter whose opening and closing delimiters are
/// each a whole line of exactly `---` (trailing whitespace or `\r` allowed).
/// Returns the frontmatter lines and the body after the closing line.
fn split_strict_frontmatter(content: &str) -> Option<(&str, &str)> {
    let first_end = content.find('\n')?;
    if content[..first_end].trim_end() != "---" {
        return None;
    }
    let rest = &content[first_end + 1..];
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let frontmatter = rest[..offset].trim_end_matches(['\n', '\r']);
            return Some((frontmatter, &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A file opening with a `----` rule has no frontmatter; everything up to
    /// a later `---` used to be rewritten as if it were.
    #[test]
    fn a_horizontal_rule_is_not_frontmatter() {
        let content = b"----\nmodel: is prose here\n---\nrest";
        let result = String::from_utf8(transform_agent_for_copilot(content)).unwrap();

        assert!(result.starts_with("---\ntarget: github-copilot\n---\n\n----\n"));
        assert!(result.contains("model: is prose here"));
    }

    /// The closing line must be exactly `---`; `---text` is body.
    #[test]
    fn a_closing_line_with_text_after_the_dashes_does_not_close() {
        let content = b"---\nname: a\n---text\nmore\n---\nbody";
        let result = String::from_utf8(transform_agent_for_copilot(content)).unwrap();

        assert!(result.ends_with("---\nbody"), "{result}");
        assert!(result.contains("---text"), "{result}");
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
