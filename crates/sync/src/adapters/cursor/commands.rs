//! Command reading and writing for Cursor adapter.
//!
//! Cursor commands are markdown files in `.cursor/commands/*.md`,
//! near-identical to Claude Code commands.

use super::paths::commands_dir;
use super::utils::{sanitize_name, strip_frontmatter};
use crate::adapters::utils::hash_content;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::debug;

/// Reads all commands from `.cursor/commands/*.md`.
pub fn read_commands(root: &Path) -> Result<Vec<Command>> {
    let dir = commands_dir(root);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut commands = Vec::new();

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

        commands.push(Command {
            name,
            content,
            source_path: path,
            modified,
            hash,
            modules: vec![],
            content_format: ContentFormat::default(),
        });
    }

    commands.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(commands)
}

/// Writes commands as `.cursor/commands/{name}.md` files.
pub fn write_commands(root: &Path, commands: &[Command]) -> Result<WriteReport> {
    let dir = commands_dir(root);
    let mut report = WriteReport::default();

    if commands.is_empty() {
        return Ok(report);
    }

    fs::create_dir_all(&dir)?;

    for cmd in commands {
        let name = sanitize_name(&cmd.name);
        let path = dir.join(format!("{}.md", name));

        // Strip all frontmatter — Cursor commands don't support YAML frontmatter.
        // Frontmatter fields (allowed-tools, description, etc.) cause Cursor to
        // display "--- (user)" instead of the command description.
        let content_str = String::from_utf8_lossy(&cmd.content);
        let body = strip_frontmatter(&content_str);
        let cursor_bytes = body.as_bytes();

        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_content(&existing) == hash_content(cursor_bytes) {
                report.skipped.push(SkipReason::Unchanged {
                    item: cmd.name.clone(),
                });
                continue;
            }
        }

        debug!(name = %name, path = ?path, "Writing Cursor command");
        fs::write(&path, cursor_bytes)?;
        report.written += 1;
    }

    Ok(report)
}
