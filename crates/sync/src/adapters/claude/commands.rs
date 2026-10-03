//! Command discovery and writing for the Claude adapter.
//!
//! Handles `~/.claude/commands/`, the plugins-cache layout, and
//! optional marketplace sources. Each commands location yields .md
//! files keyed by file stem.

use crate::adapters::utils::{hash_content, is_hidden_path, sanitize_name};
use crate::common::{Command, ContentFormat};
use crate::report::WriteReport;
use crate::Result;

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use walkdir::WalkDir;

use super::plugin_cache::plugin_cache_files;
use super::ClaudeAdapter;

pub(super) fn collect_commands_from_dir(
    dir: &Path,
    seen: &mut HashSet<String>,
    commands: &mut Vec<Command>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(dir).min_depth(1).max_depth(8) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!(error = %e, "Skipping unreadable entry under the commands directory");
                continue;
            }
        };
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

        if let Some(command) = read_command(name, path, None) {
            commands.push(command);
        }
    }

    Ok(())
}

/// Reads one command file; an unreadable file is skipped with a warning rather
/// than failing the whole sync.
fn read_command(
    name: String,
    path: &Path,
    plugin_origin: Option<crate::common::PluginOrigin>,
) -> Option<Command> {
    let read = fs::read(path).and_then(|c| Ok((c, fs::metadata(path)?)));
    let (content, metadata) = match read {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "Skipping unreadable command");
            return None;
        }
    };
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let hash = hash_content(&content);
    Some(Command {
        name,
        content,
        source_path: path.to_path_buf(),
        modified,
        hash,
        modules: Vec::new(),
        content_format: ContentFormat::default(),
        plugin_origin,
    })
}

pub(super) fn read_commands_impl(
    adapter: &ClaudeAdapter,
    include_marketplace: bool,
) -> Result<Vec<Command>> {
    let mut commands = Vec::new();
    let mut seen = HashSet::new();

    // 1) Core ~/.claude/commands
    collect_commands_from_dir(&adapter.commands_dir(), &mut seen, &mut commands)?;

    // 2) Plugin cache: the latest version of each plugin, with its origin.
    let cache_path = adapter.config_root_ref().join("plugins/cache");
    for (path, origin) in plugin_cache_files(&cache_path, "commands") {
        let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(command) = read_command(name, &path, Some(origin)) {
            commands.push(command);
        }
    }

    // 3) Marketplaces (uninstalled plugins), on request.
    if include_marketplace {
        let marketplaces_path = adapter.config_root_ref().join("plugins/marketplaces");
        collect_marketplace_commands(&marketplaces_path, &mut seen, &mut commands);
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

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for cmd in commands {
        writer.write_single(
            &cmd.name,
            &sanitize_name(&cmd.name),
            ".md",
            &cmd.content,
            &mut report,
        )?;
    }

    Ok(report)
}

/// Collects `.md` files under a `commands` directory anywhere below the
/// marketplaces checkout, skipping hidden paths and symlinks.
fn collect_marketplace_commands(
    base: &Path,
    seen: &mut HashSet<String>,
    commands: &mut Vec<Command>,
) {
    if !base.exists() {
        return;
    }
    for entry in WalkDir::new(base)
        .min_depth(1)
        .max_depth(8)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!(error = %e, "Skipping unreadable entry under plugin marketplaces");
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(base) else {
            continue;
        };
        if is_hidden_path(rel) {
            continue;
        }
        let under_commands = rel
            .parent()
            .is_some_and(|dir| dir.components().any(|c| c.as_os_str() == "commands"));
        if !under_commands {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(command) = read_command(name, path, None) {
            commands.push(command);
        }
    }
}
