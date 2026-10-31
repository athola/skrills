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
/// (`-beta`, `+sha`) is ignored. A name with any non-numeric part (a commit
/// hash such as `921c90ff926e`) gets the empty key: it ranks below every
/// release, and such names tie and fall through to the mtime tie-break.
fn version_key(name: &str) -> Vec<u64> {
    let core = name.split(['-', '+']).next().unwrap_or(name);
    core.split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}

fn utf8_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str();
    if name.is_none() {
        tracing::warn!(path = %path.display(), "Skipping non-UTF-8 plugin cache directory");
    }
    name.map(str::to_owned)
}

/// The non-hidden, non-symlink subdirectories of `dir`. Clears `complete`
/// when `dir` or one of its entries could not be read, so a caller can tell an
/// empty directory from one it failed to list.
fn subdirs(dir: &Path, complete: &mut bool) -> Vec<PathBuf> {
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry),
                Err(e) => {
                    tracing::warn!(dir = %dir.display(), error = %e, "Skipping unreadable plugin cache entry");
                    *complete = false;
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
            *complete = false;
            Vec::new()
        }
    }
}

/// Lists the one version directory to read for each cached plugin: the highest
/// version by [`version_key`], then the most recently modified, then the
/// greatest name, so the pick never depends on `read_dir` order.
pub(super) fn latest_plugin_versions(cache_dir: &Path) -> Vec<PluginVersion> {
    latest_plugin_versions_listing(cache_dir).0
}

/// [`latest_plugin_versions`], plus whether every cache directory could be
/// listed. When it is `false`, a plugin missing from the list may still be
/// installed.
pub(super) fn latest_plugin_versions_listing(cache_dir: &Path) -> (Vec<PluginVersion>, bool) {
    let mut complete = true;
    let mut picked = Vec::new();
    for publisher_dir in subdirs(cache_dir, &mut complete) {
        let Some(publisher) = utf8_name(&publisher_dir) else {
            continue;
        };
        for plugin_dir in subdirs(&publisher_dir, &mut complete) {
            let Some(plugin_name) = utf8_name(&plugin_dir) else {
                continue;
            };
            let latest = subdirs(&plugin_dir, &mut complete)
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
    (picked, complete)
}

/// Lists the `.md` files under the top-level `kind_dir` directory (`skills`,
/// `commands`, `agents`, `hooks`) in the latest version of every cached plugin, with the
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
            // Only the plugin's own top-level `kind_dir` counts: a
            // `docs/commands/` README is not a command, and an `agents/`
            // directory inside a skill holds that skill's modules.
            let under_kind = rel.components().count() > 1
                && rel
                    .components()
                    .next()
                    .is_some_and(|c| c.as_os_str() == kind_dir);
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
        assert!(version_key("1.10.0") > version_key("1.9.21"));
    }

    /// A commit-hash directory used to rank by its leading digits, so
    /// `921c90ff926e` read as version 921 and beat `1.0.1`.
    #[test]
    fn version_key_ranks_hash_names_below_every_release() {
        assert!(version_key("921c90ff926e") < version_key("1.0.1"));
        assert!(version_key("41f619d61b99") < version_key("0.0.1"));
        assert_eq!(version_key("921c90ff926e"), version_key("abc1234"));
    }

    #[test]
    fn latest_plugin_versions_picks_the_newest_hash_dir_by_mtime() {
        let tmp = tempfile::TempDir::new().unwrap();
        let plugin = tmp.path().join("official/github");
        let (old, new) = (plugin.join("921c90ff926e"), plugin.join("41f619d61b99"));
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        let past = SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::open(&old).unwrap().set_modified(past).unwrap();

        let picked = latest_plugin_versions(tmp.path());
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].origin.version, "41f619d61b99");
    }
}
