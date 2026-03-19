//! Skills reading and writing for Cursor adapter.
//!
//! Cursor skills are directories containing `SKILL.md` plus optional
//! companion files, located in `.cursor/skills/`. Unlike Claude skills,
//! Cursor skills have **no YAML frontmatter** — the content is pure markdown.
//!
//! When writing Claude skills to Cursor, YAML frontmatter is stripped.
//! When reading Cursor skills back, no frontmatter is expected.
//!
//! ## Lossy roundtrip warning
//!
//! Syncing Claude → Cursor **strips YAML frontmatter** (name, description,
//! dependencies, version, etc.). A subsequent Cursor → Claude sync will
//! **not** restore that metadata. Treat Cursor as a one-way *consumer*
//! of Claude skills; do not use it as the source of truth for skills
//! that originated in Claude.

use super::paths::skills_dir;
use super::utils::{sanitize_name, strip_frontmatter};
use crate::adapters::utils::{collect_module_files, hash_content};
use crate::common::{Command, ContentFormat};
use crate::report::WriteReport;
use crate::Result;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::debug;

/// Reads all skills from `.cursor/skills/`.
pub fn read_skills(root: &Path) -> Result<Vec<Command>> {
    let dir = skills_dir(root);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut skills = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }
        if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }

        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();

        let content = fs::read(&skill_md)?;
        let hash = hash_content(&content);
        let modified = fs::metadata(&skill_md)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);

        let modules = collect_module_files(&path);

        skills.push(Command {
            name,
            content,
            source_path: skill_md,
            modified,
            hash,
            modules,
            content_format: ContentFormat::default(),
        });
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Writes skills to `.cursor/skills/{name}/SKILL.md`.
///
/// Strips YAML frontmatter from content (Cursor skills don't use frontmatter).
pub fn write_skills(root: &Path, skills: &[Command]) -> Result<WriteReport> {
    let dir = skills_dir(root);
    let mut report = WriteReport::default();

    if skills.is_empty() {
        return Ok(report);
    }

    fs::create_dir_all(&dir)?;

    for skill in skills {
        let name = sanitize_name(&skill.name);
        let skill_dir = dir.join(&name);
        fs::create_dir_all(&skill_dir)?;

        // Strip Claude frontmatter when writing to Cursor
        let content_str = String::from_utf8_lossy(&skill.content);
        let body = strip_frontmatter(&content_str);
        let skill_path = skill_dir.join("SKILL.md");

        debug!(name = %name, path = ?skill_path, "Writing Cursor skill");
        fs::write(&skill_path, body.as_bytes())?;

        // Write companion/module files (with path containment check)
        for module in &skill.modules {
            let module_path = skill_dir.join(&module.relative_path);
            if !crate::adapters::utils::is_path_contained(&module_path, &skill_dir) {
                debug!(
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
