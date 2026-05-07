//! Command discovery and writing for the Claude adapter.
//!
//! Handles `~/.claude/commands/`, the plugins-cache layout, and
//! optional marketplace sources. Each commands location yields .md
//! files keyed by file stem.

use crate::adapters::utils::{hash_content, sanitize_name};
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use walkdir::WalkDir;

use super::ClaudeAdapter;

pub(super) fn collect_commands_from_dir(
    dir: &PathBuf,
    seen: &mut HashSet<String>,
    commands: &mut Vec<Command>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(dir).min_depth(1).max_depth(8) {
        let entry = entry?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }
        match path.extension() {
            Some(ext) if ext == "md" => {}
            _ => continue,
        }

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                tracing::warn!(
                    ?path,
                    "non-UTF-8 file stem; multiple such files will collide on the 'unknown' name"
                );
                "unknown".to_string()
            });

        if !seen.insert(name.clone()) {
            continue;
        }

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
            content_format: ContentFormat::default(),
            plugin_origin: None,
        });
    }

    Ok(())
}

pub(super) fn read_commands_impl(
    adapter: &ClaudeAdapter,
    include_marketplace: bool,
) -> Result<Vec<Command>> {
    let mut commands = Vec::new();
    let mut seen = HashSet::new();

    // 1) Core ~/.claude/commands
    collect_commands_from_dir(&adapter.commands_dir(), &mut seen, &mut commands)?;

    // 2) Marketplaces & Cache
    let mut bases: Vec<&Path> = Vec::new();
    let cache_path = adapter.config_root_ref().join("plugins/cache");
    let marketplaces_path = adapter.config_root_ref().join("plugins/marketplaces");
    bases.push(cache_path.as_path());
    if include_marketplace {
        bases.push(marketplaces_path.as_path());
    }

    for base_path in bases {
        if !base_path.exists() {
            continue;
        }
        for entry in WalkDir::new(base_path).min_depth(1).max_depth(8) {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }
            match path.extension() {
                Some(ext) if ext == "md" => {}
                _ => continue,
            }

            // Only include files that live under a commands directory
            if !path
                .ancestors()
                .any(|p| p.file_name().is_some_and(|n| n == "commands"))
            {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            if !seen.insert(name.clone()) {
                continue;
            }

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
                content_format: ContentFormat::default(),
                plugin_origin: None,
            });
        }
    }

    Ok(commands)
}

pub(super) fn write_commands_impl(
    adapter: &ClaudeAdapter,
    commands: &[Command],
) -> Result<WriteReport> {
    let dir = adapter.commands_dir();
    fs::create_dir_all(&dir)?;

    let mut report = WriteReport::default();

    for cmd in commands {
        let safe_name = sanitize_name(&cmd.name);
        let path = dir.join(format!("{}.md", safe_name));

        // Check if unchanged
        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_content(&existing) == cmd.hash {
                report.skipped.push(SkipReason::Unchanged {
                    item: cmd.name.clone(),
                });
                continue;
            }
        }

        fs::write(&path, &cmd.content)?;
        report.written += 1;
    }

    Ok(report)
}
