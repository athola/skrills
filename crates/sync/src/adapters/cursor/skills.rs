//! Skills reading and writing for Cursor adapter.
//!
//! Cursor skills are directories containing `SKILL.md` plus optional
//! companion files, located in `.cursor/skills/`. Cursor loads a skill only
//! when its frontmatter has `name` (equal to the folder name) and
//! `description` (cursor.com/docs/context/skills).
//!
//! When writing Claude skills to Cursor, the frontmatter is cut down to
//! `name`, `description` and `disable-model-invocation`, and `model_hint` is
//! kept as an HTML comment for routing. The original frontmatter is kept in a
//! trailing `<!-- skrills:frontmatter ... -->` comment, and reading the skill
//! back from Cursor restores it, so a Claude -> Cursor -> Codex sync gives
//! Codex the source's fields rather than Cursor's subset.
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
use crate::adapters::utils::{collect_module_files, hash_content};
use crate::common::{Command, ContentFormat, PluginOrigin};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use std::fs;
use std::path::Path;
use tracing::debug;

/// Writes `.cursor-plugin/plugin.json` in `plugins/local/<plugin>/` from the
/// Claude manifest in the skill's own plugin cache, or a minimal one.
///
/// The manifest is looked up beside `source_path` (the cache the skill was
/// read from), not in the real `~/.claude`. A directory skrills marked is
/// refreshed when the manifest changed (a plugin version bump); a manifest
/// in an unmarked directory is the user's and is left alone. Nothing is
/// written through a symlinked plugin directory.
fn ensure_cursor_plugin_manifest(root: &Path, origin: &PluginOrigin, source_path: &Path) {
    use crate::adapters::utils::sanitize_name;

    let safe_name = sanitize_name(&origin.plugin_name);

    let local_dir = root.join("plugins").join("local");
    let local_plugin = local_dir.join(&safe_name);
    let cursor_manifest = local_plugin.join(".cursor-plugin").join("plugin.json");
    if crate::adapters::utils::symlink_below(&local_dir, &cursor_manifest).is_some() {
        debug!(plugin = %safe_name, "Plugin directory is a symlink; leaving its manifest alone");
        return;
    }
    let exists = cursor_manifest.exists();
    if exists && !super::has_mirror_marker(&local_plugin) {
        return;
    }
    // A directory this call creates is skrills' own, so it gets the mirror
    // marker: a later full plugin mirror can then manage it instead of
    // refusing it as one the user made.
    let created_dir = !local_plugin.exists();

    let manifest_json = match cache_manifest_for(source_path, origin) {
        Some(path) => match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    plugin = %safe_name,
                    path = %path.display(),
                    error = %e,
                    "Claude manifest exists but is unreadable; using synthetic"
                );
                minimal_cursor_manifest(origin)
            }
        },
        None => minimal_cursor_manifest(origin),
    };
    if exists && fs::read_to_string(&cursor_manifest).is_ok_and(|c| c == manifest_json) {
        return;
    }

    if write_manifest_file(&cursor_manifest, &manifest_json, &safe_name) && created_dir {
        mark_as_mirror(&local_plugin, &safe_name);
    }
}

/// The Claude cache manifest of the plugin version `source_path` sits in:
/// the ancestor `<home>/plugins/cache/<publisher>/<plugin>/<version>`.
fn cache_manifest_for(source_path: &Path, origin: &PluginOrigin) -> Option<std::path::PathBuf> {
    source_path.ancestors().find_map(|dir| {
        let home = dir.ancestors().nth(5)?;
        let manifest = claude_cache_manifest_path(home, origin)?;
        (manifest == dir.join(".claude-plugin").join("plugin.json") && manifest.is_file())
            .then_some(manifest)
    })
}

fn mark_as_mirror(plugin_dir: &Path, plugin_name: &str) {
    if let Err(e) = crate::adapters::utils::write_file(
        plugin_dir.join(super::MIRROR_MARKER),
        super::MIRROR_MARKER_BODY,
    ) {
        tracing::warn!(plugin = %plugin_name, error = %e, "Could not stamp mirror marker");
    }
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

/// Writes a manifest file, creating parent directories as needed. Returns
/// whether it was written.
fn write_manifest_file(path: &std::path::Path, content: &str, plugin_name: &str) -> bool {
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(
                plugin = %plugin_name,
                path = %parent.display(),
                error = %e,
                "Could not create manifest directory"
            );
            return false;
        }
    }
    if let Err(e) = crate::adapters::utils::write_file(path, content) {
        tracing::warn!(
            plugin = %plugin_name,
            error = %e,
            "Could not write .cursor-plugin/plugin.json"
        );
        false
    } else {
        debug!(
            plugin = %plugin_name,
            "Created .cursor-plugin/plugin.json for Cursor discovery"
        );
        true
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

/// The frontmatter Cursor reads from a `SKILL.md`. `name` must equal the
/// folder name. The description is written as a JSON string, which YAML reads
/// back unchanged whatever it starts with (`[`, `*`, `-`, a quote).
fn cursor_frontmatter(
    name: &str,
    description: Option<&str>,
    disable_model_invocation: bool,
) -> String {
    let mut yaml = format!("---\nname: {name}\n");
    if let Some(desc) = description {
        let quoted = serde_json::to_string(desc).expect("a string serializes to JSON");
        yaml.push_str(&format!("description: {quoted}\n"));
    }
    if disable_model_invocation {
        yaml.push_str("disable-model-invocation: true\n");
    }
    yaml.push_str("---\n");
    yaml
}

/// Reads all skills from `.cursor/skills/`.
pub fn read_skills(root: &Path) -> Result<Vec<Command>> {
    let dir = skills_dir(root);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut skills = Vec::new();

    for entry in fs::read_dir(&dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!(error = %e, "Skipping unreadable entry under the skills directory");
                continue;
            }
        };
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

        let Some((content, modified)) = crate::adapters::utils::read_item_file(&skill_md) else {
            continue;
        };
        // A copy this crate wrote gets its original frontmatter back.
        let content = match std::str::from_utf8(&content).ok().and_then(restore_skill) {
            Some(restored) => restored.into_bytes(),
            None => content,
        };
        let hash = hash_content(&content);

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
/// Replaces the frontmatter with the fields Cursor reads (see the module docs).
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
        let name = crate::adapters::utils::skill_folder_name(skill);

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

        let body = match &model_hint {
            Some(hint) => format!("<!-- model_hint: {hint} -->\n\n{trimmed}"),
            None => trimmed,
        };
        // Cursor loads a skill only with `name` (its folder) and a
        // description, so a source with no frontmatter gets the name too.
        if description.is_none() {
            report.warnings.push(format!(
                "Skill {}: no `description`, which Cursor requires to load a skill",
                skill.name
            ));
        }
        let disable_model_invocation = fields
            .get("disable-model-invocation")
            .is_some_and(|v| strip_yaml_quotes(v) == "true");
        let body = format!(
            "{}\n{body}",
            cursor_frontmatter(&name, description.as_deref(), disable_model_invocation)
        );
        // Cursor has no place for the other Claude fields; keep the original
        // in a trailing comment so a sync out of Cursor can restore them.
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
                ensure_cursor_plugin_manifest(root, origin, &skill.source_path);
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
