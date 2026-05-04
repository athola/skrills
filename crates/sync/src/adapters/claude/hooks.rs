//! Hook discovery and writing for the Claude adapter.
//!
//! Walks `~/.claude/hooks/` and the plugins cache, keeping the most
//! recently modified version per hook name.

use crate::adapters::utils::{hash_content, is_hidden_path, sanitize_name};
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;

use std::collections::HashMap;
use std::fs;
use std::time::SystemTime;

use walkdir::WalkDir;

use super::ClaudeAdapter;

pub(super) fn read_hooks_impl(adapter: &ClaudeAdapter) -> Result<Vec<Command>> {
    // Track hooks by name, keeping the most recently modified version
    let mut hooks_map: HashMap<String, Command> = HashMap::new();

    // Helper to process a hook and update the map if it's newer
    let mut process_hook = |name: String, path: &std::path::Path| -> Result<()> {
        let content = fs::read(path)?;
        let metadata = fs::metadata(path)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let hash = hash_content(&content);

        let hook = Command {
            name: name.clone(),
            content,
            source_path: path.to_path_buf(),
            modified,
            hash,
            modules: Vec::new(),

            content_format: ContentFormat::default(),
            plugin_origin: None,
        };

        match hooks_map.get(&name) {
            Some(existing) if existing.modified >= modified => {}
            _ => {
                hooks_map.insert(name, hook);
            }
        }
        Ok(())
    };

    // 1) Core ~/.claude/hooks
    let hooks_dir = adapter.hooks_dir();
    if hooks_dir.exists() {
        for entry in WalkDir::new(&hooks_dir)
            .min_depth(1)
            .max_depth(10)
            .follow_links(false)
        {
            let entry = entry?;
            if entry.file_type().is_symlink() {
                continue;
            }
            let path = entry.path();
            if is_hidden_path(path.strip_prefix(&hooks_dir).unwrap_or(path)) {
                continue;
            }
            if !path.is_file() {
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            process_hook(name, path)?;
        }
    }

    // 2) Plugins cache ~/.claude/plugins/cache/**/hooks/
    let cache_dir = adapter.config_root_ref().join("plugins/cache");
    if cache_dir.exists() {
        for entry in WalkDir::new(&cache_dir).min_depth(1).max_depth(10) {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }

            // Only include files under a hooks directory
            if !path
                .ancestors()
                .any(|p| p.file_name().is_some_and(|n| n == "hooks"))
            {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            if name == "unknown" || name == "hooks" {
                continue;
            }

            process_hook(name, path)?;
        }
    }

    Ok(hooks_map.into_values().collect())
}

pub(super) fn write_hooks_impl(adapter: &ClaudeAdapter, hooks: &[Command]) -> Result<WriteReport> {
    let dir = adapter.hooks_dir();
    fs::create_dir_all(&dir)?;

    let mut report = WriteReport::default();

    for hook in hooks {
        let safe_name = sanitize_name(&hook.name);
        let path = dir.join(format!("{}.md", safe_name));

        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_content(&existing) == hook.hash {
                report.skipped.push(SkipReason::Unchanged {
                    item: hook.name.clone(),
                });
                continue;
            }
        }

        fs::write(&path, &hook.content)?;
        report.written += 1;
    }

    Ok(report)
}
