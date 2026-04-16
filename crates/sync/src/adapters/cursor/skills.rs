//! Skills reading and writing for Cursor adapter.
//!
//! Cursor skills are directories containing `SKILL.md` plus optional
//! companion files, located in `.cursor/skills/`. Unlike Claude skills,
//! Cursor skills have **no YAML frontmatter** — the content is pure markdown.
//!
//! When writing Claude skills to Cursor, YAML frontmatter is stripped.
//! The `description` field is preserved as a plain-text first line (Cursor
//! shows it as the skill subtitle), and `model_hint` is kept as an HTML
//! comment for routing. When reading Cursor skills back, no frontmatter
//! is expected.
//!
//! ## Lossy roundtrip warning
//!
//! Syncing Claude → Cursor **strips most YAML frontmatter** (name,
//! dependencies, version, tags, etc.). `description` and `model_hint`
//! are preserved in non-YAML form. A subsequent Cursor → Claude sync
//! will **not** restore the original metadata. Treat Cursor as a
//! one-way *consumer* of Claude skills; do not use it as the source
//! of truth for skills that originated in Claude.

use super::paths::skills_dir;
use super::utils::{parse_frontmatter, sanitize_name, strip_yaml_quotes, trim_skill_body};
use crate::adapters::utils::{collect_module_files, hash_content};
use crate::common::{Command, ContentFormat, PluginOrigin};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::debug;

/// Creates `.cursor-plugin/plugin.json` in `plugins/local/<plugin>/` if it
/// doesn't already exist (write-once). Reads the Claude manifest from the
/// cache if available, otherwise generates a minimal one.
///
/// This is deliberately write-once: if the manifest exists it is never updated.
/// The `write_plugin_assets` path handles manifest updates via hash comparison.
fn ensure_cursor_plugin_manifest(root: &Path, origin: &PluginOrigin) {
    use crate::adapters::utils::sanitize_name;

    let safe_name = sanitize_name(&origin.plugin_name);
    let safe_publisher = sanitize_name(&origin.publisher);
    let safe_version = sanitize_name(&origin.version);

    let local_plugin = root.join("plugins").join("local").join(&safe_name);
    let cursor_manifest = local_plugin.join(".cursor-plugin").join("plugin.json");
    if cursor_manifest.exists() {
        return;
    }

    // Try to read the Claude manifest from the Claude plugin cache (~/.claude/).
    let claude_home = match dirs::home_dir() {
        Some(h) => h.join(".claude"),
        None => {
            debug!("HOME not set; using synthetic manifest for {}", safe_name);
            let json = minimal_cursor_manifest(origin);
            write_manifest_file(&cursor_manifest, &json, &safe_name);
            return;
        }
    };
    let claude_manifest_path = claude_home
        .join("plugins")
        .join("cache")
        .join(&safe_publisher)
        .join(&safe_name)
        .join(&safe_version)
        .join(".claude-plugin")
        .join("plugin.json");

    let manifest_json = if claude_manifest_path.exists() {
        match fs::read_to_string(&claude_manifest_path) {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    plugin = %safe_name,
                    path = %claude_manifest_path.display(),
                    error = %e,
                    "Claude manifest exists but is unreadable; using synthetic"
                );
                minimal_cursor_manifest(origin)
            }
        }
    } else {
        minimal_cursor_manifest(origin)
    };

    write_manifest_file(&cursor_manifest, &manifest_json, &safe_name);
}

/// Writes a manifest file, creating parent directories as needed.
fn write_manifest_file(path: &std::path::Path, content: &str, plugin_name: &str) {
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(
                plugin = %plugin_name,
                path = %parent.display(),
                error = %e,
                "Could not create manifest directory"
            );
            return;
        }
    }
    if let Err(e) = fs::write(path, content) {
        tracing::warn!(
            plugin = %plugin_name,
            error = %e,
            "Could not write .cursor-plugin/plugin.json"
        );
    } else {
        debug!(
            plugin = %plugin_name,
            "Created .cursor-plugin/plugin.json for Cursor discovery"
        );
    }
}

/// Generates a minimal plugin.json when the Claude manifest is unavailable.
fn minimal_cursor_manifest(origin: &PluginOrigin) -> String {
    serde_json::json!({
        "name": origin.plugin_name,
        "version": origin.version,
        "description": format!("Synced from {} via skrills", origin.publisher)
    })
    .to_string()
}

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
            plugin_origin: None,
        });
    }

    skills.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(skills)
}

/// Writes skills to `.cursor/skills/{name}/SKILL.md` or, when the skill has
/// a [`PluginOrigin`], to `.cursor/plugins/local/{plugin}/skills/{name}/SKILL.md`
/// so that Cursor's plugin system discovers them as installed plugins.
///
/// Strips YAML frontmatter from content (Cursor skills don't use frontmatter).
pub fn write_skills(root: &Path, skills: &[Command]) -> Result<WriteReport> {
    let flat_dir = skills_dir(root);
    let local_plugins_dir = root.join("plugins").join("local");
    let mut report = WriteReport::default();

    if skills.is_empty() {
        return Ok(report);
    }

    fs::create_dir_all(&flat_dir)?;

    // Track which plugins we've written so we can create their manifests once
    let mut seen_plugins: std::collections::HashSet<String> = std::collections::HashSet::new();

    for skill in skills {
        let name = sanitize_name(&skill.name);

        // Decide write target: plugin-local dir if origin is known, flat dir otherwise
        let skill_dir = if let Some(ref origin) = skill.plugin_origin {
            let plugin_dir = local_plugins_dir.join(&origin.plugin_name);
            // Ensure .cursor-plugin/plugin.json manifest exists (once per plugin)
            if seen_plugins.insert(origin.plugin_name.clone()) {
                ensure_cursor_plugin_manifest(root, origin);
            }
            plugin_dir.join("skills").join(&name)
        } else {
            flat_dir.join(&name)
        };
        fs::create_dir_all(&skill_dir)?;

        // Parse Claude frontmatter to extract metadata before stripping
        let content_str = String::from_utf8_lossy(&skill.content);
        let (fields, raw_body) = parse_frontmatter(&content_str);

        let description = fields.get("description").map(|d| strip_yaml_quotes(d));
        let model_hint = fields.get("model_hint").cloned();

        // Trim non-essential sections from the body
        let trimmed = trim_skill_body(raw_body);

        // Inject description as plain text (Cursor shows the first line as
        // the skill subtitle) and model_hint as an HTML comment for routing.
        let mut header = String::new();
        if let Some(desc) = &description {
            header.push_str(desc);
            header.push('\n');
        }
        if let Some(hint) = &model_hint {
            header.push_str(&format!("<!-- model_hint: {hint} -->\n"));
        }
        let body = if header.is_empty() {
            trimmed
        } else {
            format!("{header}\n{trimmed}")
        };

        let skill_path = skill_dir.join("SKILL.md");

        let skill_unchanged = if skill_path.exists() {
            let existing = fs::read(&skill_path)?;
            hash_content(&existing) == hash_content(body.as_bytes())
        } else {
            false
        };

        if skill_unchanged && skill.modules.is_empty() {
            report.skipped.push(SkipReason::Unchanged {
                item: skill.name.clone(),
            });
            continue;
        }

        if !skill_unchanged {
            debug!(name = %name, path = ?skill_path, "Writing Cursor skill");
            fs::write(&skill_path, body.as_bytes())?;
        }

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
