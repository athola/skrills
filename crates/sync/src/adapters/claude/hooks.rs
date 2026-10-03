//! Hook discovery and writing for the Claude adapter.
//!
//! Walks `~/.claude/hooks/` and the plugins cache, keeping the most
//! recently modified version per hook name.

use crate::adapters::utils::{hash_content, is_hidden_path, sanitize_name};
use crate::common::{Command, ContentFormat};
use crate::report::WriteReport;
use crate::Result;

use std::collections::HashMap;
use std::fs;
use std::time::SystemTime;

use walkdir::WalkDir;

use super::plugin_cache::plugin_cache_files;
use super::ClaudeAdapter;

pub(super) fn read_hooks_impl(adapter: &ClaudeAdapter) -> Result<Vec<Command>> {
    // Track hooks by name, keeping the most recently modified version
    let mut hooks_map: HashMap<String, Command> = HashMap::new();

    // Helper to process a hook and update the map if it's newer
    let mut process_hook =
        |name: String,
         path: &std::path::Path,
         plugin_origin: Option<crate::common::PluginOrigin>| {
            let read = fs::read(path).and_then(|c| Ok((c, fs::metadata(path)?)));
            let (content, metadata) = match read {
                Ok(pair) => pair,
                Err(e) => {
                    // One unreadable file used to abort the whole sync.
                    tracing::warn!(path = %path.display(), error = %e, "Skipping unreadable hook");
                    return;
                }
            };
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
                plugin_origin,
            };

            match hooks_map.get(&name) {
                Some(existing) if existing.modified >= modified => {}
                _ => {
                    hooks_map.insert(name, hook);
                }
            }
        };

    // 1) Core ~/.claude/hooks
    let hooks_dir = adapter.hooks_dir();
    if hooks_dir.exists() {
        for entry in WalkDir::new(&hooks_dir)
            .min_depth(1)
            .max_depth(10)
            .follow_links(false)
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping unreadable entry under the hooks directory");
                    continue;
                }
            };
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

            process_hook(name, path, None);
        }
    }

    // 2) Plugin cache: the latest version of each plugin, with its origin.
    let cache_dir = adapter.config_root_ref().join("plugins/cache");
    for (path, origin) in plugin_cache_files(&cache_dir, "hooks") {
        let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        process_hook(name, &path, Some(origin));
    }

    Ok(hooks_map.into_values().collect())
}

pub(super) fn write_hooks_impl(adapter: &ClaudeAdapter, hooks: &[Command]) -> Result<WriteReport> {
    let dir = adapter.hooks_dir();
    fs::create_dir_all(&dir)?;

    let mut report = WriteReport::default();

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for hook in hooks {
        if hook.content_format == ContentFormat::Json {
            // Claude Code takes hooks from the `hooks` key of settings.json;
            // a JSON entry list parked in hooks/<Event>.md is never loaded.
            report
                .skipped
                .push(crate::report::SkipReason::AgentSpecificFeature {
                    item: hook.name.clone(),
                    feature: "JSON hook entries (Cursor hooks.json)".to_string(),
                    suggestion:
                        "Add the hook to the `hooks` section of ~/.claude/settings.json by hand"
                            .to_string(),
                });
            continue;
        }
        writer.write_single(
            &hook.name,
            &sanitize_name(&hook.name),
            ".md",
            &hook.content,
            &mut report,
        )?;
    }

    Ok(report)
}
