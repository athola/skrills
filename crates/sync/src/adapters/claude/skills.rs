//! Skill discovery and writing for the Claude adapter.
//!
//! Split out of `claude/mod.rs` (FU-2). Combines:
//! - `read_skills_impl` — walks both `~/.claude/skills/` and the
//!   plugins-cache layout `plugins/cache/<pub>/<plugin>/<ver>/`,
//!   keeping the most-recently-modified version per skill name.
//! - `write_skills_impl` — emits SKILL.md inside per-skill
//!   directories with companion module files alongside.

use crate::adapters::utils::{collect_module_files, hash_content, is_hidden_path, sanitize_name};
use crate::common::{Command, ContentFormat, ModuleFile};
use crate::report::{SkipReason, WriteReport};
use crate::Result;

use std::collections::HashMap;
use std::fs;
use std::time::SystemTime;

use walkdir::WalkDir;

use super::ClaudeAdapter;

pub(super) fn read_skills_impl(adapter: &ClaudeAdapter) -> Result<Vec<Command>> {
    // Track skills by name, keeping the most recently modified version
    let mut skills_map: HashMap<String, Command> = HashMap::new();

    // Helper to process a skill and update the map if it's newer
    let process_skill =
        |skills_map: &mut HashMap<String, Command>,
         name: String,
         path: &std::path::Path,
         modules: Vec<ModuleFile>,
         plugin_origin: Option<crate::common::PluginOrigin>| {
            let content = match fs::read(path) {
                Ok(c) => c,
                Err(_) => return,
            };
            let metadata = match fs::metadata(path) {
                Ok(m) => m,
                Err(_) => return,
            };
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let hash = hash_content(&content);

            let skill = Command {
                name: name.clone(),
                content,
                source_path: path.to_path_buf(),
                modified,
                hash,
                modules,
                content_format: ContentFormat::default(),
                plugin_origin,
            };

            // Keep the most recently modified version
            match skills_map.get(&name) {
                Some(existing) if existing.modified >= modified => {
                    // Existing is newer or same, skip
                }
                _ => {
                    skills_map.insert(name, skill);
                }
            }
        };

    // 1) Core ~/.claude/skills
    let skills_dir = adapter.skills_dir();
    if skills_dir.exists() {
        for entry in WalkDir::new(&skills_dir)
            .min_depth(1)
            .max_depth(20)
            .follow_links(false)
        {
            let entry = entry?;
            if entry.file_type().is_symlink() {
                continue;
            }
            let path = entry.path();
            if is_hidden_path(path.strip_prefix(&skills_dir).unwrap_or(path)) {
                continue;
            }
            if !path.is_file() {
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "md") {
                continue;
            }

            let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
            let (name, modules) = if is_skill_md {
                let name = path
                    .parent()
                    .and_then(|p| p.strip_prefix(&skills_dir).ok())
                    .and_then(|p| p.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unknown")
                    .to_string();
                // Collect companion files from the skill directory
                let skill_dir = path.parent().unwrap_or(path);
                let modules = collect_module_files(skill_dir);
                (name, modules)
            } else {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                (name, Vec::new())
            };

            process_skill(&mut skills_map, name, path, modules, None);
        }
    }

    // 2) Plugins cache ~/.claude/plugins/cache/**/*
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

            // Only include files under a skills directory
            if !path
                .ancestors()
                .any(|p| p.file_name().is_some_and(|n| n == "skills"))
            {
                continue;
            }

            // Extract skill name from path
            let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
            let (name, modules) = if is_skill_md {
                // Use parent directory name as skill name
                let name = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .filter(|s| !s.is_empty() && *s != "skills")
                    .unwrap_or("unknown")
                    .to_string();
                // Collect companion files from the skill directory
                let skill_dir = path.parent().unwrap_or(path);
                let modules = collect_module_files(skill_dir);
                (name, modules)
            } else {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();
                (name, Vec::new())
            };

            if name == "unknown" || name == "skills" {
                continue;
            }

            // Derive plugin origin from the cache path structure:
            // plugins/cache/<publisher>/<plugin>/<version>/skills/...
            let origin = path.strip_prefix(&cache_dir).ok().and_then(|rel| {
                let mut components = rel.components();
                let publisher = components.next()?.as_os_str().to_str()?.to_string();
                let plugin_name = components.next()?.as_os_str().to_str()?.to_string();
                let version = components.next()?.as_os_str().to_str()?.to_string();
                Some(crate::common::PluginOrigin {
                    plugin_name,
                    publisher,
                    version,
                })
            });

            process_skill(&mut skills_map, name, path, modules, origin);
        }
    }

    Ok(skills_map.into_values().collect())
}

pub(super) fn write_skills_impl(
    adapter: &ClaudeAdapter,
    skills: &[Command],
) -> Result<WriteReport> {
    let dir = adapter.skills_dir();
    fs::create_dir_all(&dir)?;

    let mut report = WriteReport::default();

    for skill in skills {
        // Claude is permissive, but writing Codex-style SKILL.md keeps skills portable.
        let skill_rel_dir = if skill.name.eq_ignore_ascii_case("skill")
            || skill.name.eq_ignore_ascii_case("skill.md")
            || skill.name.eq_ignore_ascii_case("SKILL")
        {
            skill
                .source_path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or(&skill.name)
                .to_string()
        } else {
            skill.name.clone()
        };

        let safe_rel_dir = sanitize_name(&skill_rel_dir);
        let path = dir.join(&safe_rel_dir).join("SKILL.md");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Check if unchanged
        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_content(&existing) == skill.hash {
                report.skipped.push(SkipReason::Unchanged {
                    item: skill.name.clone(),
                });
                continue;
            }
        }

        fs::write(&path, &skill.content)?;

        // Write module files (companion files) alongside SKILL.md
        let skill_dir = dir.join(&safe_rel_dir);
        for module in &skill.modules {
            let module_path = skill_dir.join(&module.relative_path);
            if !crate::adapters::utils::is_path_contained(&module_path, &skill_dir) {
                tracing::debug!(
                    path = %module.relative_path.display(),
                    "Skipping module with path outside skill directory"
                );
                continue;
            }
            if let Some(parent) = module_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&module_path, &module.content)?;
        }

        report.written += 1;
    }

    Ok(report)
}
