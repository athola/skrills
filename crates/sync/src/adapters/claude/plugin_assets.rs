//! Plugin-cache asset walker for the Claude adapter.
//!
//! Split out of `claude/mod.rs`, the largest single trait method in the adapter
//! (~205 LOC) and pure recursion over the plugin cache layout
//! `cache/<marketplace>/<plugin>/<version>/`. It carries its own
//! filtering rules (synced vs skipped directories) which are easier
//! to read in isolation than embedded in the trait impl.
//!
//! The trait method [`super::ClaudeAdapter::read_plugin_assets`]
//! delegates here.

use crate::common::PluginAsset;
use crate::Result;

use std::fs;

use walkdir::WalkDir;

use super::plugin_cache::latest_plugin_versions;
use super::ClaudeAdapter;
use crate::adapters::utils::is_hidden_path;

/// Walk the plugin cache and return every asset file as a [`PluginAsset`].
///
/// In `full_mirror` mode (used for Cursor sync), include everything
/// except a small dev-noise list. Otherwise skip directories already
/// covered by other sync paths (`skills`, `commands`, `agents`) plus
/// the dev-noise list.
pub(super) fn read_plugin_assets_impl(
    adapter: &ClaudeAdapter,
    full_mirror: bool,
) -> Result<Vec<PluginAsset>> {
    let cache_dir = adapter.config_root_ref().join("plugins/cache");
    if !cache_dir.exists() {
        return Ok(vec![]);
    }

    // In normal mode, skip dirs handled by other sync paths.
    // In full_mirror mode, include everything (for targets like Cursor
    // that need a complete plugin cache copy).
    let synced_dirs: &[&str] = if full_mirror {
        &[]
    } else {
        &["skills", "commands", "agents"]
    };
    // Directories to skip (not needed at runtime)
    let skip_dirs: &[&str] = if full_mirror {
        &["tests", ".venv", "__pycache__", "node_modules", ".git"]
    } else {
        &[
            "tests",
            ".venv",
            "__pycache__",
            "node_modules",
            ".git",
            ".claude-plugin",
            ".cursor-plugin",
        ]
    };

    let mut assets = Vec::new();

    // One version per plugin, picked by the same rule as every other reader.
    for version_dir in latest_plugin_versions(&cache_dir) {
        let version_path = version_dir.path;
        let crate::common::PluginOrigin {
            plugin_name,
            publisher,
            version,
        } = version_dir.origin;
        let assets_before = assets.len();

        // Walk the version directory collecting asset files
        for entry in WalkDir::new(&version_path)
            .min_depth(1)
            .max_depth(10)
            .follow_links(false)
        {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(
                        plugin = %plugin_name,
                        error = %e,
                        "Failed to read directory entry while scanning plugin assets"
                    );
                    continue;
                }
            };
            let path = entry.path();

            if !path.is_file() {
                continue;
            }
            // A link may point anywhere on disk; mirroring its target
            // would copy that file into another tool's plugin tree.
            if entry.path_is_symlink()
                && !crate::adapters::utils::is_path_contained(path, &version_path)
            {
                tracing::warn!(
                    path = %path.display(),
                    plugin = %plugin_name,
                    "Skipping plugin asset symlink that points outside the plugin"
                );
                continue;
            }
            if let Ok(meta) = fs::metadata(path) {
                if meta.len() > crate::adapters::utils::MAX_MODULE_FILE_BYTES {
                    tracing::warn!(
                        path = %path.display(),
                        plugin = %plugin_name,
                        bytes = meta.len(),
                        "Skipping plugin asset over the size limit"
                    );
                    continue;
                }
            }

            let rel_path = match path.strip_prefix(&version_path) {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Skip hidden files, but in full_mirror mode, allow
            // .claude-plugin/ (plugin manifests needed by targets like Cursor)
            if is_hidden_path(rel_path)
                && (!full_mirror
                    || rel_path
                        .components()
                        .next()
                        .is_none_or(|c| c.as_os_str() != ".claude-plugin"))
            {
                continue;
            }

            // Check if this file is under a synced or skipped directory
            let top_component = rel_path
                .components()
                .next()
                .and_then(|c| c.as_os_str().to_str())
                .unwrap_or("");

            if synced_dirs.contains(&top_component) {
                continue; // Already synced by skills/commands/agents
            }
            if skip_dirs.contains(&top_component) {
                continue;
            }
            // Also check any ancestor for skip dirs (e.g., nested __pycache__)
            if rel_path.components().any(|c| {
                c.as_os_str()
                    .to_str()
                    .is_some_and(|s| skip_dirs.contains(&s))
            }) {
                continue;
            }

            let content = match fs::read(path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        plugin = %plugin_name,
                        error = %e,
                        "Failed to read plugin asset file, skipping"
                    );
                    continue;
                }
            };

            let executable = {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::metadata(path)
                        .map(|m| m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                }
                #[cfg(not(unix))]
                {
                    false
                }
            };

            assets.push(PluginAsset::new(
                plugin_name.clone(),
                publisher.clone(),
                version.clone(),
                rel_path.to_path_buf(),
                content,
                executable,
            ));
        }

        if full_mirror {
            // Use a structurally-built path so the comparison is correct on
            // Windows (where `strip_prefix` produces backslash separators)
            // as well as Unix. A `to_string_lossy().contains("/")` check
            // would silently miss real Windows manifests and double-emit a
            // synthetic one.
            let manifest_rel_path = std::path::Path::new(".claude-plugin").join("plugin.json");
            let has_manifest = assets[assets_before..]
                .iter()
                .any(|a| a.relative_path == manifest_rel_path);
            if !has_manifest {
                let description = derive_description(&version_path, &plugin_name);
                let manifest = synthesize_manifest(&plugin_name, &version, &description);
                tracing::info!(
                    plugin = %plugin_name,
                    version = %version,
                    "Synthesized .claude-plugin/plugin.json for manifest-less plugin"
                );
                assets.push(PluginAsset::new(
                    plugin_name.clone(),
                    publisher.clone(),
                    version.clone(),
                    manifest_rel_path,
                    manifest,
                    false,
                ));
            }
        }
    }

    Ok(assets)
}

/// Build a minimal `plugin.json` body for plugins that ship without one.
///
/// Loaders only need name and version to recognize the plugin; the empty
/// component arrays make it explicit that this plugin contributes no
/// commands/skills/agents/hooks (matching the on-disk reality for
/// e.g. typescript-lsp / pyright-lsp, which are MCP-server-only).
fn synthesize_manifest(name: &str, version: &str, description: &str) -> Vec<u8> {
    let body = serde_json::json!({
        "name": name,
        "version": version,
        "description": description,
        "commands": [],
        "skills": [],
        "agents": [],
        "hooks": [],
        "_synthesized": true,
    });
    // Serialization of a `serde_json::Value` built from primitive fields
    // cannot fail. Using `expect` instead of `unwrap_or_else` ensures any
    // future refactor that breaks this invariant surfaces loudly rather
    // than emitting a 1-byte file that masquerades as a valid manifest.
    let mut out =
        serde_json::to_vec_pretty(&body).expect("static JSON value cannot fail to serialize");
    out.push(b'\n');
    out
}

/// Pull a one-line description from README.md (first non-empty,
/// non-heading paragraph). Falls back to a generic string so the
/// manifest always has a `description` field.
fn derive_description(version_path: &std::path::Path, plugin_name: &str) -> String {
    let readme = version_path.join("README.md");
    if let Ok(text) = fs::read_to_string(&readme) {
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Stop at the first prose line; truncate to keep manifests compact.
            return line.chars().take(280).collect();
        }
    }
    format!("Plugin '{plugin_name}' (manifest synthesized by skrills sync)")
}
