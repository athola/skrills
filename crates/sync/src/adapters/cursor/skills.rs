//! Skills reading and writing for Cursor adapter.
//!
//! Cursor skills are directories containing `SKILL.md` plus optional
//! companion files, located in `.cursor/skills/`. Unlike Claude skills,
//! Cursor skills have **no YAML frontmatter**, the content is pure markdown.
//!
//! When writing Claude skills to Cursor, YAML frontmatter is stripped.
//! The `description` field is preserved as a plain-text first line (Cursor
//! shows it as the skill subtitle), and `model_hint` is kept as an HTML
//! comment for routing. The original frontmatter is kept in a trailing
//! `<!-- skrills:frontmatter ... -->` comment, and reading the skill back
//! from Cursor restores it, so a Claude -> Cursor -> Codex sync still gives
//! Codex a `SKILL.md` with `name:` and `description:`.
//!
//! ## Lossy roundtrip warning
//!
//! The body is still trimmed on the way in: the "Supporting Modules",
//! "See Also" and "Table of Contents" sections and `modules/...` link lines
//! are dropped and are not restored by a sync back out of Cursor. A skill
//! written in Cursor itself carries no stash and is read back as it is.

use super::paths::skills_dir;
use super::utils::{
    parse_frontmatter, restore_skill, stash_frontmatter, strip_yaml_quotes, trim_skill_body,
};
use crate::adapters::utils::sanitize_name_kebab;
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
    let claude_manifest_path = claude_cache_manifest_path(&claude_home, origin);

    let manifest_json = if let Some(claude_manifest_path) =
        claude_manifest_path.as_deref().filter(|p| p.exists())
    {
        match fs::read_to_string(claude_manifest_path) {
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

/// Where the Claude cache keeps this plugin's manifest.
///
/// The components are the cache directory names exactly as read, so they are
/// used verbatim: sanitizing turned version `1.2.3` into `123`, so the real
/// manifest was never found and a synthetic one always written. A component
/// that could leave the cache (empty, `.`, `..`, or holding a separator)
/// yields `None`.
fn claude_cache_manifest_path(
    claude_home: &Path,
    origin: &PluginOrigin,
) -> Option<std::path::PathBuf> {
    let parts = [&origin.publisher, &origin.plugin_name, &origin.version];
    let safe =
        |part: &str| !part.is_empty() && part != "." && part != ".." && !part.contains(['/', '\\']);
    if !parts.iter().all(|p| safe(p)) {
        return None;
    }
    let mut path = claude_home.join("plugins").join("cache");
    for part in parts {
        path.push(part);
    }
    Some(path.join(".claude-plugin").join("plugin.json"))
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
    if let Err(e) = crate::adapters::utils::write_file(path, content) {
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
        // A copy this crate wrote gets its original frontmatter back.
        let content = match std::str::from_utf8(&content).ok().and_then(restore_skill) {
            Some(restored) => restored.into_bytes(),
            None => content,
        };
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

    let mut flat_writer = crate::adapters::utils::BatchWriter::new(&flat_dir);
    let mut plugin_writer = crate::adapters::utils::BatchWriter::new(&local_plugins_dir);

    for skill in skills {
        let name = sanitize_name_kebab(&skill.name);

        // A lossy decode wrote U+FFFD into the copy; skip instead.
        let content_str = match std::str::from_utf8(&skill.content) {
            Ok(s) => s,
            Err(e) => {
                report.skipped.push(SkipReason::ParseError {
                    item: skill.name.clone(),
                    error: format!("not valid UTF-8: {e}"),
                });
                continue;
            }
        };
        // Parse Claude frontmatter to extract metadata before stripping
        let (fields, raw_body) = parse_frontmatter(content_str);
        let (raw_yaml, _, _) = skrills_validate::frontmatter::split_frontmatter(content_str);

        let description = fields.get("description").map(|d| strip_yaml_quotes(d));
        let model_hint = fields.get("model_hint").cloned();

        // Trim non-essential sections from the body
        let trimmed = trim_skill_body(&raw_body);

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
        // Cursor skills have no frontmatter; keep the original in a trailing
        // comment so a sync out of Cursor can restore `name:` and the rest.
        let body = match raw_yaml.as_deref() {
            None => body,
            Some(yaml) => stash_frontmatter(&body, yaml).unwrap_or_else(|| {
                report.warnings.push(format!(
                    "Skill {}: its frontmatter could not be kept for a sync back out of \
                     Cursor (it contains `-->`)",
                    skill.name
                ));
                body
            }),
        };

        // Decide write target: plugin-local dir if origin is known, flat dir otherwise
        if let Some(ref origin) = skill.plugin_origin {
            // Sanitized with the same function `ensure_cursor_plugin_manifest`
            // uses, so the manifest and the skill body share one directory. The
            // raw name also came straight from upstream plugin metadata, so
            // joining it unfiltered let `../` steer the write out of
            // `plugins/local/`.
            let safe_plugin = crate::adapters::utils::sanitize_name(&origin.plugin_name);
            if safe_plugin.is_empty() || name.is_empty() {
                report.skipped.push(SkipReason::Refused {
                    item: skill.name.clone(),
                    reason: "the plugin or skill name sanitizes to an empty directory name"
                        .to_string(),
                });
                continue;
            }
            // Ensure .cursor-plugin/plugin.json manifest exists (once per plugin)
            if seen_plugins.insert(origin.plugin_name.clone()) {
                ensure_cursor_plugin_manifest(root, origin);
            }
            debug!(name = %name, plugin = %safe_plugin, "Writing Cursor plugin skill");
            plugin_writer.write(
                &skill.name,
                &format!("{safe_plugin}/skills/{name}"),
                "SKILL.md",
                body.as_bytes(),
                &skill.modules,
                &mut report,
            )?;
        } else {
            debug!(name = %name, "Writing Cursor skill");
            flat_writer.write(
                &skill.name,
                &name,
                "SKILL.md",
                body.as_bytes(),
                &skill.modules,
                &mut report,
            )?;
        }
    }

    Ok(report)
}

#[cfg(test)]
mod manifest_path_tests {
    use super::*;

    fn origin(publisher: &str, plugin: &str, version: &str) -> PluginOrigin {
        PluginOrigin {
            plugin_name: plugin.to_string(),
            publisher: publisher.to_string(),
            version: version.to_string(),
        }
    }

    #[test]
    fn the_cache_manifest_path_keeps_the_dots_in_the_version() {
        let path = claude_cache_manifest_path(
            Path::new("/h/.claude"),
            &origin("mp", "my.plugin", "1.2.3"),
        )
        .unwrap();
        assert_eq!(
            path,
            Path::new("/h/.claude/plugins/cache/mp/my.plugin/1.2.3/.claude-plugin/plugin.json")
        );
    }

    #[test]
    fn a_component_that_leaves_the_cache_is_refused() {
        let home = Path::new("/h/.claude");
        assert!(claude_cache_manifest_path(home, &origin("..", "p", "1")).is_none());
        assert!(claude_cache_manifest_path(home, &origin("mp", "a/b", "1")).is_none());
        assert!(claude_cache_manifest_path(home, &origin("mp", "p", "")).is_none());
    }
}
