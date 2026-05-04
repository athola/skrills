//! Agent discovery and writing for the Claude adapter.
//!
//! Walks `~/.claude/agents/` and the plugins cache, keeping the
//! most recently modified version per agent name. Mirrors the
//! shape of the hooks module but targets the `agents` directory.

use crate::adapters::utils::{hash_content, is_hidden_path, sanitize_name};
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;

use std::collections::HashMap;
use std::fs;
use std::time::SystemTime;

use walkdir::WalkDir;

use super::ClaudeAdapter;

pub(super) fn read_agents_impl(adapter: &ClaudeAdapter) -> Result<Vec<Command>> {
    // Track agents by name, keeping the most recently modified version
    let mut agents_map: HashMap<String, Command> = HashMap::new();

    // Helper to process an agent and update the map if it's newer
    let mut process_agent = |name: String, path: &std::path::Path| -> Result<()> {
        let content = fs::read(path)?;
        let metadata = fs::metadata(path)?;
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
            plugin_origin: None,
        };

        match agents_map.get(&name) {
            Some(existing) if existing.modified >= modified => {}
            _ => {
                agents_map.insert(name, agent);
            }
        }
        Ok(())
    };

    // 1) Core ~/.claude/agents
    let agents_dir = adapter.agents_dir();
    if agents_dir.exists() {
        for entry in WalkDir::new(&agents_dir)
            .min_depth(1)
            .max_depth(10)
            .follow_links(false)
        {
            let entry = entry?;
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

            process_agent(name, path)?;
        }
    }

    // 2) Plugins cache ~/.claude/plugins/cache/**/agents/
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

            // Only include files under an agents directory
            if !path
                .ancestors()
                .any(|p| p.file_name().is_some_and(|n| n == "agents"))
            {
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            if name == "unknown" || name == "agents" {
                continue;
            }

            process_agent(name, path)?;
        }
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

    for agent in agents {
        let safe_name = sanitize_name(&agent.name);
        let path = dir.join(format!("{}.md", safe_name));

        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_content(&existing) == agent.hash {
                report.skipped.push(SkipReason::Unchanged {
                    item: agent.name.clone(),
                });
                continue;
            }
        }

        fs::write(&path, &agent.content)?;
        report.written += 1;
    }

    Ok(report)
}
