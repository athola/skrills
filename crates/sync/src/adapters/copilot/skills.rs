//! Skills reading and writing for Copilot adapter.

use super::paths::skills_dir;
use super::utils::sanitize_name;
use crate::adapters::utils::{hash_content, is_hidden_path};
use crate::common::{Command, ModuleFile};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use anyhow::Context;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::{debug, warn};
use walkdir::WalkDir;

/// Collects companion files from a skill directory (files other than SKILL.md).
pub fn collect_module_files(skill_dir: &Path) -> Vec<ModuleFile> {
    let mut modules = Vec::new();

    for entry in WalkDir::new(skill_dir)
        .min_depth(1)
        .max_depth(10)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                debug!(
                    error = %e,
                    path = ?e.path(),
                    "Skipping directory entry due to traversal error"
                );
                continue;
            }
        };

        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        // Skip the main skill file - we want companion/module files only.
        // Note: We treat any .md file in the skill directory (other than SKILL.md)
        // as a module file. If a .md file should be a standalone skill, it should
        // be placed in its own directory with a SKILL.md file instead.
        if path.file_name().is_some_and(|n| n == "SKILL.md") {
            continue;
        }

        if let Ok(rel_path) = path.strip_prefix(skill_dir) {
            if is_hidden_path(rel_path) {
                continue;
            }

            if let Ok(content) = fs::read(path) {
                let hash = hash_content(&content);
                modules.push(ModuleFile {
                    relative_path: rel_path.to_path_buf(),
                    content,
                    hash,
                });
            }
        }
    }

    modules
}

/// Reads skills from the skills directory.
pub fn read_skills(root: &Path) -> Result<Vec<Command>> {
    let skills_dir = skills_dir(root);
    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    let mut skills = Vec::new();
    for entry in WalkDir::new(&skills_dir)
        .min_depth(1)
        .max_depth(20)
        .follow_links(false)
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    path = ?e.path(),
                    error = %e,
                    "Failed to read directory entry while scanning skills"
                );
                continue;
            }
        };
        if entry.file_type().is_symlink() {
            continue;
        }
        let path = entry.path();
        if is_hidden_path(path.strip_prefix(&skills_dir).unwrap_or(path)) {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }

        // Copilot skills are discovered via ~/.copilot/skills/**/SKILL.md
        let is_skill_md = path.file_name().is_some_and(|n| n == "SKILL.md");
        if !is_skill_md {
            continue;
        }

        // Use the parent directory path relative to skills_dir as the skill identifier.
        // Example: ~/.copilot/skills/pdf-processing/SKILL.md -> "pdf-processing"
        // Example: ~/.copilot/skills/nested/foo/SKILL.md -> "nested/foo"
        let name = path
            .parent()
            .and_then(|p| p.strip_prefix(&skills_dir).ok())
            .and_then(|p| p.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_string();

        let content = fs::read(path)?;
        let metadata = fs::metadata(path)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let hash = hash_content(&content);

        // Collect companion files from the skill directory
        let skill_dir = path.parent().unwrap_or(path);
        let modules = collect_module_files(skill_dir);

        skills.push(Command {
            name,
            content,
            source_path: path.to_path_buf(),
            modified,
            hash,
            modules,
        });
    }
    Ok(skills)
}

/// Writes skills to the skills directory.
pub fn write_skills(root: &Path, skills: &[Command]) -> Result<WriteReport> {
    let dir = skills_dir(root);
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create skills directory: {}", dir.display()))?;

    let mut report = WriteReport::default();

    for skill in skills {
        // Write each skill into ~/.copilot/skills/<skill-name>/SKILL.md
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
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create skill directory: {}", parent.display())
            })?;
        }

        if path.exists() {
            let existing = fs::read(&path)
                .with_context(|| format!("Failed to read existing skill: {}", path.display()))?;
            if hash_content(&existing) == skill.hash {
                report.skipped.push(SkipReason::Unchanged {
                    item: skill.name.clone(),
                });
                continue;
            }
        }

        fs::write(&path, &skill.content)
            .with_context(|| format!("Failed to write skill: {}", path.display()))?;

        // Write module files (companion files) alongside SKILL.md
        let skill_dir = dir.join(&safe_rel_dir);
        for module in &skill.modules {
            let module_path = skill_dir.join(&module.relative_path);
            if let Some(parent) = module_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("Failed to create module directory: {}", parent.display())
                })?;
            }
            fs::write(&module_path, &module.content).with_context(|| {
                format!("Failed to write module file: {}", module_path.display())
            })?;
        }

        report.written += 1;
    }

    // Note: Unlike Codex, Copilot does NOT require config.toml feature flags

    Ok(report)
}
