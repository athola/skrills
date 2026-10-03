//! Agent reading and writing for Cursor adapter.
//!
//! Cursor agents are markdown files with YAML frontmatter in `.cursor/agents/`.
//! Key differences from Claude agents:
//! - `background: true` → `is_background: true`
//! - `tools` and `isolation` fields are Claude-only: stripped from the Cursor
//!   frontmatter, reported as a warning, and kept in a trailing
//!   `<!-- skrills:frontmatter ... -->` comment that reading the agent back
//!   from Cursor puts into the frontmatter again
//! - `is_background` is not renamed back to `background` on read
//! - `readonly` is Cursor-only (preserved on read)
//! - `model` is copied unchanged (not translated)

use super::paths::agents_dir;
use super::utils::{stash_frontmatter, take_stash};
use crate::adapters::utils::hash_content;
use crate::adapters::utils::sanitize_name_kebab;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

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
        // A copy this crate wrote gets its Claude-only lines back.
        let content = match std::str::from_utf8(&content).ok().and_then(restore_agent) {
            Some(restored) => restored.into_bytes(),
            None => content,
        };
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
            plugin_origin: None,
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

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for agent in agents {
        let name = sanitize_name_kebab(&agent.name);

        // Translate frontmatter fields
        let content_str = match std::str::from_utf8(&agent.content) {
            Ok(s) => s,
            Err(e) => {
                report.skipped.push(SkipReason::ParseError {
                    item: agent.name.clone(),
                    error: format!("not valid UTF-8: {e}"),
                });
                continue;
            }
        };
        let (translated, dropped) = translate_agent_frontmatter(content_str);
        let translated = if dropped.is_empty() {
            translated
        } else {
            stash_frontmatter(&translated, &dropped).unwrap_or(translated)
        };
        if dropped.lines().any(|l| l.starts_with("isolation:")) {
            report.warnings.push(format!(
                "Agent {} sets `isolation` in Claude; Cursor agents have no such field, so \
                 the Cursor copy runs without it.",
                agent.name
            ));
        }
        if restricts_tools(content_str) {
            report.warnings.push(format!(
                "Agent {} limits its tools in Claude; Cursor agents have no tool list, so it \
                 can use every tool there. Add `readonly: true` to the Cursor copy if it \
                 should not write.",
                agent.name
            ));
        }

        writer.write_single(
            &agent.name,
            &name,
            ".md",
            translated.as_bytes(),
            &mut report,
        )?;
    }

    Ok(report)
}

/// Whether the agent's frontmatter has a `tools:` key, the restriction
/// [`translate_agent_frontmatter`] has to drop.
fn restricts_tools(content: &str) -> bool {
    let (raw, _body, _line) = skrills_validate::frontmatter::split_frontmatter(content);
    raw.is_some_and(|fm| fm.lines().any(|line| line.starts_with("tools:")))
}

/// Puts the Claude-only lines a Cursor copy stashed back at the end of its
/// frontmatter. `None` when the file carries no stash.
fn restore_agent(content: &str) -> Option<String> {
    let (body, dropped) = take_stash(content)?;
    let (raw, rest, _) = skrills_validate::frontmatter::split_frontmatter(body);
    let fm = raw?;
    let mut out = format!("---\n{fm}\n{dropped}\n---\n");
    if !rest.is_empty() {
        out.push('\n');
        out.push_str(&rest);
        out.push('\n');
    }
    Some(out)
}

/// Translates Claude agent frontmatter to Cursor conventions.
///
/// - Renames `background` → `is_background`
/// - Strips `tools` and `isolation` fields (not supported by Cursor)
/// - Passes through all other fields unchanged
///
/// Returns the translated file and the stripped lines (empty when none).
fn translate_agent_frontmatter(content: &str) -> (String, String) {
    let (raw_frontmatter, body, _line) = skrills_validate::frontmatter::split_frontmatter(content);

    let Some(frontmatter_str) = raw_frontmatter else {
        return (content.to_string(), String::new());
    };

    let mut translated_lines = Vec::new();
    let mut dropped_lines: Vec<&str> = Vec::new();
    let mut skipping_block = false;

    for line in frontmatter_str.lines() {
        let trimmed_line = line.trim();

        // Skip Claude-only fields and their multi-line continuations
        if trimmed_line.starts_with("tools:") || trimmed_line.starts_with("isolation:") {
            skipping_block = true;
            dropped_lines.push(line);
            continue;
        }

        // If we were skipping a block field, continue skipping continuation lines
        // (indented lines or list items that belong to the previous field)
        if skipping_block {
            if line.starts_with(' ') || line.starts_with('\t') {
                dropped_lines.push(line);
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
        result.push_str(&body);
    }
    (result, dropped_lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translate_background_to_is_background() {
        let input = "---\nname: reviewer\nbackground: true\nmodel: claude-sonnet-4-6\n---\n\nReview code.\n";
        let output = translate_agent_frontmatter(input).0;
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
        let output = translate_agent_frontmatter(input).0;
        assert!(!output.contains("tools:"));
        assert!(!output.contains("isolation:"));
        assert!(output.contains("name: builder"));
        assert!(output.contains("model: opus"));
    }

    #[test]
    fn translate_strips_multiline_tools_list() {
        let input = "---\nname: builder\ntools:\n  - Read\n  - Write\n  - Bash\nmodel: opus\n---\n\nBuild things.\n";
        let output = translate_agent_frontmatter(input).0;
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
        let output = translate_agent_frontmatter(input).0;
        assert_eq!(output, input);
    }

    /// T3: translate_agent_frontmatter produces the expected output format:
    /// ---\nfield: value\n---\n\nbody
    #[test]
    fn translate_agent_frontmatter_output_format() {
        let input =
            "---\nname: test-agent\ndescription: A test agent\nmodel: opus\n---\n\nDo the work.\n";
        let output = translate_agent_frontmatter(input).0;

        // Must start with opening delimiter
        assert!(
            output.starts_with("---\n"),
            "Output must start with '---\\n', got: {:?}",
            &output[..20.min(output.len())]
        );

        // Must have closing delimiter
        let after_open = &output[4..]; // skip "---\n"
        let close_pos = after_open.find("\n---\n");
        assert!(
            close_pos.is_some(),
            "Output must contain closing '\\n---\\n' delimiter"
        );

        // Extract frontmatter between delimiters
        let fm = &after_open[..close_pos.unwrap()];
        assert!(
            fm.contains("name: test-agent"),
            "Frontmatter must preserve name"
        );
        assert!(
            fm.contains("description: A test agent"),
            "Frontmatter must preserve description"
        );
        assert!(
            fm.contains("model: opus"),
            "Frontmatter must preserve model"
        );

        // Body must follow after the closing delimiter
        let body_start = close_pos.unwrap() + 5; // skip "\n---\n"
        let body = &after_open[body_start..];
        assert!(
            body.contains("Do the work."),
            "Body must be preserved after frontmatter, got: {:?}",
            body
        );
    }
}
