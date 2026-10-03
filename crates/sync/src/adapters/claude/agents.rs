//! Agent discovery and writing for the Claude adapter.
//!
//! Walks `~/.claude/agents/` and the plugins cache, keeping the
//! most recently modified version per agent name. Mirrors the
//! shape of the hooks module but targets the `agents` directory.

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

pub(super) fn read_agents_impl(adapter: &ClaudeAdapter) -> Result<Vec<Command>> {
    // Track agents by name, keeping the most recently modified version
    let mut agents_map: HashMap<String, Command> = HashMap::new();

    // Helper to process an agent and update the map if it's newer
    let mut process_agent =
        |name: String,
         path: &std::path::Path,
         plugin_origin: Option<crate::common::PluginOrigin>| {
            let read = fs::read(path).and_then(|c| Ok((c, fs::metadata(path)?)));
            let (content, metadata) = match read {
                Ok(pair) => pair,
                Err(e) => {
                    // One unreadable file used to abort the whole sync.
                    tracing::warn!(path = %path.display(), error = %e, "Skipping unreadable agent");
                    return;
                }
            };
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            let agent = Command {
                name: name.clone(),
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
                modules: Vec::new(),

                content_format: ContentFormat::default(),
                plugin_origin,
            };

            match agents_map.get(&name) {
                Some(existing) if existing.modified >= modified => {}
                _ => {
                    agents_map.insert(name, agent);
                }
            }
        };

    // 1) Core ~/.claude/agents
    let agents_dir = adapter.agents_dir();
    if agents_dir.exists() {
        for entry in WalkDir::new(&agents_dir)
            .min_depth(1)
            .max_depth(10)
            .follow_links(false)
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(error = %e, "Skipping unreadable entry under the agents directory");
                    continue;
                }
            };
            if entry.file_type().is_symlink() {
                continue;
            }
            let path = entry.path();
            if is_hidden_path(path.strip_prefix(&agents_dir).unwrap_or(path)) {
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

            process_agent(name, path, None);
        }
    }

    // 2) Plugin cache: the latest version of each plugin, with its origin.
    let cache_dir = adapter.config_root_ref().join("plugins/cache");
    for (path, origin) in plugin_cache_files(&cache_dir, "agents") {
        let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        process_agent(name, &path, Some(origin));
    }

    Ok(agents_map.into_values().collect())
}

pub(super) fn write_agents_impl(
    adapter: &ClaudeAdapter,
    agents: &[Command],
) -> Result<WriteReport> {
    let dir = adapter.agents_dir();
    fs::create_dir_all(&dir)?;

    let mut report = WriteReport::default();

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for agent in agents {
        writer.write_single(
            &agent.name,
            &sanitize_name(&agent.name),
            ".md",
            &agent.content,
            &mut report,
        )?;
    }

    Ok(report)
}
