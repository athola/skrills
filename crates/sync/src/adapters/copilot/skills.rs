//! Skills reading and writing for Copilot adapter.

use super::paths::skills_dir;
use crate::adapters::utils::{
    collect_module_files, hash_content, is_hidden_path, sanitize_name_segments,
};
use crate::common::{Command, ContentFormat};
use crate::report::WriteReport;
use crate::Result;
use anyhow::Context;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::warn;
use walkdir::WalkDir;

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
            content_format: ContentFormat::default(),
            plugin_origin: None,
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

    let mut writer = crate::adapters::utils::BatchWriter::new(&dir);
    for skill in skills {
        // Write each skill into ~/.copilot/skills/<skill-name>/SKILL.md
        let safe_rel_dir = sanitize_name_segments(&crate::adapters::utils::skill_dir_name(skill));
        writer
            .write(
                &skill.name,
                &safe_rel_dir,
                "SKILL.md",
                &skill.content,
                &skill.modules,
                &mut report,
            )
            .with_context(|| format!("Failed to write skill {}", skill.name))?;
    }

    // Note: Unlike Codex, Copilot does NOT require config.toml feature flags

    Ok(report)
}
