//! Agent reading and writing for Cursor adapter.
//!
//! Cursor agents are markdown files with YAML frontmatter in `.cursor/agents/`.
//! Key differences from Claude agents:
//! - `background: true` → `is_background: true`
//! - `tools` and `isolation` fields are Claude-only (stripped on write)
//! - `readonly` is Cursor-only (preserved on read)
//! - Model names translated via `transform_model`

use super::paths::agents_dir;
use super::utils::{sanitize_name, split_frontmatter};
use crate::adapters::utils::hash_content;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::debug;

/// Reads all agents from `.cursor/agents/*.md`.
pub fn read_agents(root: &Path) -> Result<Vec<Command>> {
    let dir = agents_dir(root);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut agents = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }

        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "md" {
            continue;
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let content = fs::read(&path)?;
        let hash = hash_content(&content);
        let modified = fs::metadata(&path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        agents.push(Command {
            name,
            content,
            source_path: path,
            modified,
            hash,
            modules: vec![],
            content_format: ContentFormat::default(),
        });
    }

    agents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(agents)
}

/// Writes agents to `.cursor/agents/{name}.md`.
///
/// Translates Claude agent frontmatter to Cursor conventions:
/// - `background: true` → `is_background: true`
/// - `tools` and `isolation` fields are stripped
pub fn write_agents(root: &Path, agents: &[Command]) -> Result<WriteReport> {
    let dir = agents_dir(root);
    let mut report = WriteReport::default();

    if agents.is_empty() {
        return Ok(report);
    }

    fs::create_dir_all(&dir)?;

    for agent in agents {
        let name = sanitize_name(&agent.name);
        let path = dir.join(format!("{}.md", name));

        // Translate frontmatter fields
        let content_str = String::from_utf8_lossy(&agent.content);
        let translated = translate_agent_frontmatter(&content_str);

        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_content(&existing) == hash_content(translated.as_bytes()) {
                report.skipped.push(SkipReason::Unchanged {
                    item: agent.name.clone(),
                });
                continue;
            }
        }

        debug!(name = %name, path = ?path, "Writing Cursor agent");
        fs::write(&path, translated.as_bytes())?;
        report.written += 1;
    }

    Ok(report)
}

/// Translates Claude agent frontmatter to Cursor conventions.
///
/// - Renames `background` → `is_background`
/// - Strips `tools` and `isolation` fields (not supported by Cursor)
/// - Passes through all other fields unchanged
fn translate_agent_frontmatter(content: &str) -> String {
    let (raw_frontmatter, body) = split_frontmatter(content);

    let Some(frontmatter_str) = raw_frontmatter else {
        return content.to_string();
    };

    let mut translated_lines = Vec::new();
    let mut skipping_block = false;

    for line in frontmatter_str.lines() {
        let trimmed_line = line.trim();

        // Skip Claude-only fields and their multi-line continuations
        if trimmed_line.starts_with("tools:") || trimmed_line.starts_with("isolation:") {
            skipping_block = true;
            continue;
        }

        // If we were skipping a block field, continue skipping continuation lines
        // (indented lines or list items that belong to the previous field)
        if skipping_block {
            if line.starts_with(' ') || line.starts_with('\t') {
                continue;
            }
            // Non-continuation line: stop skipping
            skipping_block = false;
        }

        // Rename background → is_background
        if trimmed_line.starts_with("background:") {
            let value = trimmed_line
                .strip_prefix("background:")
                .unwrap_or("false")
                .trim();
            translated_lines.push(format!("is_background: {}", value));
        } else {
            translated_lines.push(line.to_string());
        }
    }

    let mut result = String::from("---\n");
    result.push_str(&translated_lines.join("\n"));
    result.push_str("\n---\n");
    if !body.is_empty() {
        result.push('\n');
        result.push_str(body);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_background_to_is_background() {
        let input = "---\nname: reviewer\nbackground: true\nmodel: claude-sonnet-4-6\n---\n\nReview code.\n";
        let output = translate_agent_frontmatter(input);
        assert!(output.contains("is_background: true"));
        // Verify the standalone "background:" key is gone (not just a substring match)
        for line in output.lines() {
            let trimmed = line.trim();
            assert!(
                !trimmed.starts_with("background:"),
                "Found standalone 'background:' line: {}",
                trimmed
            );
        }
        assert!(output.contains("name: reviewer"));
    }

    #[test]
    fn translate_strips_tools_and_isolation() {
        let input = "---\nname: builder\ntools: [Read, Write, Bash]\nisolation: worktree\nmodel: opus\n---\n\nBuild things.\n";
        let output = translate_agent_frontmatter(input);
        assert!(!output.contains("tools:"));
        assert!(!output.contains("isolation:"));
        assert!(output.contains("name: builder"));
        assert!(output.contains("model: opus"));
    }

    #[test]
    fn translate_strips_multiline_tools_list() {
        let input = "---\nname: builder\ntools:\n  - Read\n  - Write\n  - Bash\nmodel: opus\n---\n\nBuild things.\n";
        let output = translate_agent_frontmatter(input);
        assert!(!output.contains("tools:"), "tools: should be stripped");
        assert!(
            !output.contains("  - Read"),
            "tool list items should be stripped"
        );
        assert!(
            !output.contains("  - Write"),
            "tool list items should be stripped"
        );
        assert!(
            !output.contains("  - Bash"),
            "tool list items should be stripped"
        );
        assert!(output.contains("name: builder"));
        assert!(output.contains("model: opus"));
    }

    #[test]
    fn translate_no_frontmatter_passthrough() {
        let input = "# Just markdown\n\nNo frontmatter here.\n";
        let output = translate_agent_frontmatter(input);
        assert_eq!(output, input);
    }
}
