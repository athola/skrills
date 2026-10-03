//! Command reading and writing for Cursor adapter.
//!
//! Cursor commands are markdown files in `.cursor/commands/*.md`,
//! near-identical to Claude Code commands.

use super::paths::commands_dir;
use super::utils::{restore_command, stash_frontmatter, strip_frontmatter};
use crate::adapters::utils::hash_content;
use crate::adapters::utils::sanitize_name_kebab;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

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
        // A copy this crate wrote gets its original frontmatter back.
        let content = match std::str::from_utf8(&content).ok().and_then(restore_command) {
            Some(restored) => restored.into_bytes(),
            None => content,
        };
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
            plugin_origin: None,
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

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for cmd in commands {
        let name = sanitize_name_kebab(&cmd.name);

        // Strip all frontmatter, Cursor commands don't support YAML frontmatter.
        // Frontmatter fields (allowed-tools, description, etc.) cause Cursor to
        // display "--- (user)" instead of the command description.
        // The original frontmatter rides in a trailing comment so a sync out
        // of Cursor can restore it.
        let content_str = match std::str::from_utf8(&cmd.content) {
            Ok(s) => s,
            Err(e) => {
                report.skipped.push(SkipReason::ParseError {
                    item: cmd.name.clone(),
                    error: format!("not valid UTF-8: {e}"),
                });
                continue;
            }
        };
        let body = strip_frontmatter(content_str);
        let (raw_yaml, _, _) = skrills_validate::frontmatter::split_frontmatter(content_str);
        let body = match raw_yaml.as_deref() {
            None => body,
            Some(yaml) => stash_frontmatter(&body, yaml).unwrap_or_else(|| {
                report.warnings.push(format!(
                    "Command {}: its frontmatter could not be kept for a sync back out of \
                     Cursor (it contains `-->`)",
                    cmd.name
                ));
                body
            }),
        };
        let cursor_bytes = body.as_bytes();

        writer.write_single(&cmd.name, &name, ".md", cursor_bytes, &mut report)?;
    }

    Ok(report)
}
