//! Commands (prompts) reading and writing for Copilot adapter.

use super::paths::prompts_dir;
use crate::adapters::utils::{hash_content, is_hidden_path, sanitize_name_segments};
use crate::common::{Command, ContentFormat};
use crate::report::WriteReport;
use crate::Result;
use anyhow::Context;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::warn;
use walkdir::WalkDir;

/// Suffix of a Copilot / VS Code prompt file.
const PROMPT_SUFFIX: &str = ".prompt.md";
/// Suffix earlier releases wrote by mistake; such files are still read.
const LEGACY_PROMPT_SUFFIX: &str = ".prompts.md";

/// Reads commands (prompts) from the prompts directory.
pub fn read_commands(root: &Path, _include_marketplace: bool) -> Result<Vec<Command>> {
    // Copilot uses prompts (*.prompt.md) as the equivalent of slash commands
    let prompts_dir = prompts_dir(root);
    if !prompts_dir.exists() {
        return Ok(Vec::new());
    }

    let mut commands: Vec<Command> = Vec::new();
    for entry in WalkDir::new(&prompts_dir)
        .min_depth(1)
        .max_depth(10)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    path = ?e.path(),
                    error = %e,
                    "Failed to read directory entry while scanning prompts"
                );
                continue;
            }
        };
        if entry.file_type().is_symlink() {
            continue;
        }
        let path = entry.path();
        if is_hidden_path(path.strip_prefix(&prompts_dir).unwrap_or(path)) {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        // Copilot prompts are *.prompt.md files; *.prompts.md is the old,
        // mistaken suffix this crate used to write.
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let (name, legacy) = match file_name.strip_suffix(PROMPT_SUFFIX) {
            Some(name) => (name.to_string(), false),
            None => match file_name.strip_suffix(LEGACY_PROMPT_SUFFIX) {
                Some(name) => (name.to_string(), true),
                None => continue,
            },
        };
        let rel_dir = path
            .parent()
            .and_then(|p| p.strip_prefix(&prompts_dir).ok())
            .unwrap_or(Path::new(""));
        let name = if rel_dir.as_os_str().is_empty() {
            name
        } else {
            format!("{}/{name}", rel_dir.display())
        };
        if let Some(existing) = commands.iter().position(|c| c.name == name) {
            // Both suffixes exist for one prompt: the current one wins.
            if legacy {
                continue;
            }
            commands.remove(existing);
        }

        let read = fs::read(path).and_then(|c| Ok((c, fs::metadata(path)?)));
        let (content, metadata) = match read {
            Ok(pair) => pair,
            Err(e) => {
                warn!(path = %path.display(), error = %e, "Skipping unreadable prompt");
                continue;
            }
        };
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let hash = hash_content(&content);

        commands.push(Command {
            name,
            content,
            source_path: path.to_path_buf(),
            modified,
            hash,
            modules: Vec::new(),
            content_format: ContentFormat::default(),
            plugin_origin: None,
        });
    }
    Ok(commands)
}

/// Writes commands (prompts) to the prompts directory.
pub fn write_commands(root: &Path, commands: &[Command]) -> Result<WriteReport> {
    // Copilot uses prompts (*.prompt.md) as the equivalent of slash commands
    let dir = prompts_dir(root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create prompts directory: {}", dir.display()))?;

    let mut report = WriteReport::default();

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for cmd in commands {
        writer.write_single(
            &cmd.name,
            &sanitize_name_segments(&cmd.name),
            PROMPT_SUFFIX,
            &cmd.content,
            &mut report,
        )?;
    }

    Ok(report)
}
