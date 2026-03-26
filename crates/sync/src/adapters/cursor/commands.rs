//! Command reading and writing for Cursor adapter.
//!
//! Cursor commands are markdown files in `.cursor/commands/*.md`,
//! near-identical to Claude Code commands.

use super::paths::commands_dir;
use super::utils::{parse_frontmatter, render_frontmatter, sanitize_name};
use crate::adapters::utils::hash_content;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::collections::HashMap;
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

        // Translate Claude frontmatter to Cursor format:
        // Keep only `description`; strip Claude-specific fields like
        // `allowed-tools`, `disable-model-invocation`, etc.
        let content_str = String::from_utf8_lossy(&cmd.content);
        let (fields, body) = parse_frontmatter(&content_str);
        let cursor_content = if fields.is_empty() {
            content_str.into_owned()
        } else {
            let mut cursor_fields = HashMap::new();
            if let Some(desc) = fields.get("description") {
                cursor_fields.insert("description".to_string(), desc.clone());
            }
            render_frontmatter(&cursor_fields, body)
        };
        let cursor_bytes = cursor_content.as_bytes();

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
