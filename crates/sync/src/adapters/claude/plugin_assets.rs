//! Plugin-cache asset walker for the Claude adapter.
//!
//! Split out of `claude/mod.rs` (T3.3 of the v0.8.0 refinement
//! plan) — this is the largest single trait method in the adapter
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

use super::semver_tuple;
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

    // Walk: cache/<marketplace>/<plugin>/<version>/
    for marketplace_entry in fs::read_dir(&cache_dir)? {
        let marketplace_entry = marketplace_entry?;
        let marketplace_path = marketplace_entry.path();
        if !marketplace_path.is_dir() {
            continue;
        }
        let publisher = match marketplace_path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name.to_string(),
            None => {
                tracing::warn!(
                    path = %marketplace_path.display(),
                    "Skipping non-UTF-8 marketplace directory"
                );
                continue;
            }
        };

        for plugin_entry in fs::read_dir(&marketplace_path)? {
            let plugin_entry = plugin_entry?;
            let plugin_path = plugin_entry.path();
            if !plugin_path.is_dir() {
                continue;
            }
            let plugin_name = match plugin_path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => {
                    tracing::warn!(
                        path = %plugin_path.display(),
                        "Skipping non-UTF-8 plugin directory"
                    );
                    continue;
                }
            };

            // Find the latest version directory (prefer semver, fall back to mtime)
            let mut versions: Vec<_> = fs::read_dir(&plugin_path)?
                .filter_map(|e| match e {
                    Ok(entry) => Some(entry),
                    Err(err) => {
                        tracing::warn!(
                            plugin = %plugin_name,
                            error = %err,
                            "Failed to read version directory entry"
                        );
                        None
                    }
                })
                .filter(|e| e.path().is_dir())
                .collect();
            versions.sort_by(|a, b| {
                let ver_a = semver_tuple(a);
                let ver_b = semver_tuple(b);
                ver_a.cmp(&ver_b)
            });
            let version_entry = match versions.last() {
                Some(e) => e,
                None => continue,
            };
            let version_path = version_entry.path();
            let version = match version_path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => {
                    tracing::warn!(
                        path = %version_path.display(),
                        "Skipping non-UTF-8 version directory"
                    );
                    continue;
                }
            };

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

                let rel_path = match path.strip_prefix(&version_path) {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                // Skip hidden files — but in full_mirror mode, allow
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
        }
    }

    Ok(assets)
}
