//! Commands (prompts) reading and writing for Copilot adapter.

use super::paths::prompts_dir;
use super::utils::sanitize_name;
use crate::adapters::utils::{hash_content, is_hidden_path};
use crate::common::Command;
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use anyhow::Context;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::warn;
use walkdir::WalkDir;

/// Reads commands (prompts) from the prompts directory.
pub fn read_commands(root: &Path, _include_marketplace: bool) -> Result<Vec<Command>> {
    // Copilot uses prompts (*.prompts.md) as the equivalent of slash commands
    let prompts_dir = prompts_dir(root);
    if !prompts_dir.exists() {
        return Ok(Vec::new());
    }

    let mut commands = Vec::new();
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

        // Copilot prompts are *.prompts.md files
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !file_name.ends_with(".prompts.md") {
            continue;
        }

        // Extract name: strip .prompts.md suffix
        let name = file_name.trim_end_matches(".prompts.md").to_string();

        let content = fs::read(path)?;
        let metadata = fs::metadata(path)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let hash = hash_content(&content);

        commands.push(Command {
            name,
            content,
            source_path: path.to_path_buf(),
            modified,
            hash,
            modules: Vec::new(),
        });
    }
    Ok(commands)
}

/// Writes commands (prompts) to the prompts directory.
pub fn write_commands(root: &Path, commands: &[Command]) -> Result<WriteReport> {
    // Copilot uses prompts (*.prompts.md) as the equivalent of slash commands
    let dir = prompts_dir(root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create prompts directory: {}", dir.display()))?;

    let mut report = WriteReport::default();

    for cmd in commands {
        let safe_name = sanitize_name(&cmd.name);
        let path = dir.join(format!("{}.prompts.md", safe_name));

        if path.exists() {
            let existing = fs::read(&path)
                .with_context(|| format!("Failed to read existing prompt: {}", path.display()))?;
            if hash_content(&existing) == cmd.hash {
                report.skipped.push(SkipReason::Unchanged {
                    item: cmd.name.clone(),
                });
                continue;
            }
        }

        fs::write(&path, &cmd.content)
            .with_context(|| format!("Failed to write prompt: {}", path.display()))?;
        report.written += 1;
    }

    Ok(report)
}
