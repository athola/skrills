//! One walk of the plugin cache shared by every Claude reader.
//!
//! Layout: `plugins/cache/<publisher>/<plugin>/<version>/`. Skills, commands,
//! agents, hooks and plugin assets each used to walk this tree on their own,
//! with different rules: assets took the highest semver directory, skills and
//! agents the newest file by mtime across every version, commands whichever
//! the walk met first. None of the item walks skipped hidden paths or
//! symlinks, and none recorded which plugin an item came from, so
//! `exclude_plugins` could not filter commands, agents or hooks.

use crate::adapters::utils::is_hidden_path;
use crate::common::PluginOrigin;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use walkdir::WalkDir;

/// The version directory sync reads for one cached plugin.
pub(super) struct PluginVersion {
    pub(super) origin: PluginOrigin,
    pub(super) path: PathBuf,
}

/// Orders version directory names numerically by their dot-separated leading
/// integers, so `1.10.0` sorts after `1.9.0`. A pre-release or build suffix
/// (`-beta`, `+sha`) is ignored and non-numeric parts count as 0, so such
/// names tie and fall through to the mtime tie-break.
fn version_key(name: &str) -> Vec<u64> {
    let core = name.split(['-', '+']).next().unwrap_or(name);
    core.split('.')
        .map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().unwrap_or(0)
        })
        .collect()
}

fn utf8_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str();
    if name.is_none() {
        tracing::warn!(path = %path.display(), "Skipping non-UTF-8 plugin cache directory");
    }
    name.map(str::to_owned)
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry),
                Err(e) => {
                    tracing::warn!(dir = %dir.display(), error = %e, "Skipping unreadable plugin cache entry");
                    None
                }
            })
            // `file_type` does not follow links: a symlinked plugin or version
            // directory could point anywhere.
            .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
            .map(|entry| entry.path())
            .filter(|path| !path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
            .collect(),
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "Could not read plugin cache directory");
            Vec::new()
        }
    }
}

/// Lists the one version directory to read for each cached plugin: the highest
/// version by [`version_key`], then the most recently modified, then the
/// greatest name, so the pick never depends on `read_dir` order.
pub(super) fn latest_plugin_versions(cache_dir: &Path) -> Vec<PluginVersion> {
    let mut picked = Vec::new();
    for publisher_dir in subdirs(cache_dir) {
        let Some(publisher) = utf8_name(&publisher_dir) else {
            continue;
        };
        for plugin_dir in subdirs(&publisher_dir) {
            let Some(plugin_name) = utf8_name(&plugin_dir) else {
                continue;
            };
            let latest = subdirs(&plugin_dir)
                .into_iter()
                .filter_map(|path| Some((utf8_name(&path)?, path)))
                .max_by_key(|(name, path)| {
                    let mtime = fs::metadata(path)
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    (version_key(name), mtime, name.clone())
                });
            if let Some((version, path)) = latest {
                picked.push(PluginVersion {
                    origin: PluginOrigin {
                        plugin_name: plugin_name.clone(),
                        publisher: publisher.clone(),
                        version,
                    },
                    path,
                });
            }
        }
    }
    picked
}

/// Lists the `.md` files under a `kind_dir` directory (`skills`, `commands`,
/// `agents`, `hooks`) in the latest version of every cached plugin, with the
/// plugin each came from. Hidden paths, symlinks and unreadable entries are
/// skipped; the last two with a warning.
pub(super) fn plugin_cache_files(cache_dir: &Path, kind_dir: &str) -> Vec<(PathBuf, PluginOrigin)> {
    let mut files = Vec::new();
    for PluginVersion { origin, path } in latest_plugin_versions(cache_dir) {
        for entry in WalkDir::new(&path)
            .min_depth(1)
            .max_depth(8)
            .follow_links(false)
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!(plugin = %origin.plugin_name, error = %e, "Skipping unreadable plugin cache entry");
                    continue;
                }
            };
            if entry.file_type().is_symlink() {
                tracing::debug!(path = %entry.path().display(), "Skipping symlink in plugin cache");
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let file = entry.path();
            if file.extension().is_none_or(|ext| ext != "md") {
                continue;
            }
            let Ok(rel) = file.strip_prefix(&path) else {
                continue;
            };
            if is_hidden_path(rel) {
                continue;
            }
            // Only the path inside the plugin counts: a `skills` directory
            // somewhere above the cache must not admit every file.
            let under_kind = rel
                .parent()
                .is_some_and(|dir| dir.components().any(|c| c.as_os_str() == kind_dir));
            if under_kind {
                files.push((file.to_path_buf(), origin.clone()));
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_key_orders_numerically() {
        assert!(version_key("1.10.0") > version_key("1.9.0"));
        assert!(version_key("2.0.0") > version_key("1.99.99"));
        assert_eq!(version_key("1.2.3"), version_key("1.2.3-beta"));
        assert_eq!(version_key("abc1234"), vec![0]);
    }
}
