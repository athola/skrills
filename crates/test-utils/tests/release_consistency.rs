//! Filesystem-walking release-consistency tests.
//!
//! These tests scan the actual on-disk workspace (rather than hardcoded
//! fixtures) so a real version drift across crates or between
//! `Cargo.toml` and `plugin.json` fails CI before it ships.
//!
//! Pattern adopted from `claude-night-market` v1.9.3
//! (`plugins/sanctum/tests/test_release_consistency.py`), adapted for a
//! Rust cargo workspace + `plugin.json` plugin manifest layout.
//!
//! The five invariants:
//! 1. All `crates/*/Cargo.toml` versions agree.
//! 2. `plugins/skrills/.claude-plugin/plugin.json` version matches the
//!    workspace crate version.
//! 3. Every command path in `plugin.json.commands` exists on disk.
//! 4. Top-level `plugins/skrills/commands/*.md` count equals
//!    `plugin.json.commands.length` (the `-maxdepth 1` analog from
//!    night-market — guards against helper sub-files inflating counts
//!    if a future commands/<name>/modules/ subdir is introduced).
//! 5. `.claude-plugin/marketplace.json` plugin entries (and optional
//!    `metadata.version`) agree with the workspace, and each entry's
//!    `source` path exists on disk.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points to crates/test-utils; walk up two.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root exists above crates/test-utils")
        .to_path_buf()
}

fn crates_dir() -> PathBuf {
    workspace_root().join("crates")
}

fn plugin_dir() -> PathBuf {
    workspace_root().join("plugins").join("skrills")
}

fn plugin_json_path() -> PathBuf {
    plugin_dir().join(".claude-plugin").join("plugin.json")
}

fn marketplace_json_path() -> PathBuf {
    workspace_root()
        .join(".claude-plugin")
        .join("marketplace.json")
}

/// Canonical workspace version, asserted to be uniform across all crates.
/// Panics if crates disagree — that's invariant #1's job; this helper
/// short-circuits for the tests that depend on a single value.
fn canonical_workspace_version() -> String {
    let crate_versions = collect_crate_versions();
    // Use owned Strings in the dedup set so the iterator doesn't borrow
    // from `crate_versions` and we can return a single value cleanly.
    let unique: std::collections::BTreeSet<String> = crate_versions.values().cloned().collect();
    assert_eq!(
        unique.len(),
        1,
        "crates disagree; cannot derive canonical version: {:#?}",
        crate_versions
    );
    unique.into_iter().next().expect("at least one crate")
}

fn read_crate_version(cargo_toml: &Path) -> String {
    let text = fs::read_to_string(cargo_toml)
        .unwrap_or_else(|e| panic!("read {}: {e}", cargo_toml.display()));
    let parsed: toml::Table = text
        .parse()
        .unwrap_or_else(|e| panic!("parse {}: {e}", cargo_toml.display()));
    parsed
        .get("package")
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("no [package].version in {}", cargo_toml.display()))
        .to_string()
}

fn collect_crate_versions() -> BTreeMap<String, String> {
    let mut versions = BTreeMap::new();
    for entry in fs::read_dir(crates_dir()).expect("read crates/") {
        let entry = entry.expect("read entry");
        let path = entry.path();
        let cargo_toml = path.join("Cargo.toml");
        if !cargo_toml.exists() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .expect("crate dir name is valid UTF-8");
        versions.insert(name, read_crate_version(&cargo_toml));
    }
    versions
}

fn read_plugin_json() -> serde_json::Value {
    let text =
        fs::read_to_string(plugin_json_path()).unwrap_or_else(|e| panic!("read plugin.json: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse plugin.json: {e}"))
}

#[test]
fn all_crate_versions_agree() {
    let versions = collect_crate_versions();
    assert!(
        !versions.is_empty(),
        "expected at least one crate under crates/, found none"
    );
    let unique: std::collections::BTreeSet<&String> = versions.values().collect();
    assert_eq!(
        unique.len(),
        1,
        "crate version drift across workspace: {:#?}",
        versions
    );
}

#[test]
fn plugin_json_version_matches_workspace_crates() {
    let plugin_json = read_plugin_json();
    let plugin_version = plugin_json
        .get("version")
        .and_then(|v| v.as_str())
        .expect("plugin.json has string `version`");
    let workspace_version = canonical_workspace_version();

    assert_eq!(
        plugin_version, workspace_version,
        "plugin.json version `{plugin_version}` != workspace crate version `{workspace_version}`"
    );
}

#[test]
fn plugin_json_commands_exist_on_disk() {
    let plugin_json = read_plugin_json();
    let commands = plugin_json
        .get("commands")
        .and_then(|c| c.as_array())
        .expect("plugin.json has `commands` array");

    let plugin_dir = plugin_dir();
    let mut missing: Vec<String> = Vec::new();
    for cmd in commands {
        let rel = cmd.as_str().expect("each commands entry is a string path");
        // Strip leading "./" the same way Claude Code's plugin loader does.
        let normalized = rel.strip_prefix("./").unwrap_or(rel);
        let abs = plugin_dir.join(normalized);
        if !abs.exists() {
            missing.push(rel.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "commands referenced in plugin.json but missing on disk: {missing:?}"
    );
}

#[test]
fn plugin_command_count_matches_top_level_disk() {
    let plugin_json = read_plugin_json();
    let registered = plugin_json
        .get("commands")
        .and_then(|c| c.as_array())
        .expect("plugin.json has `commands` array")
        .len();

    let commands_dir = plugin_dir().join("commands");
    // Top-level only — analog of `find commands/ -maxdepth 1 -name '*.md'`.
    // If a future commands/<name>/modules/ helper layout appears, helper
    // sub-files must not inflate the canonical command count.
    let on_disk = fs::read_dir(&commands_dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", commands_dir.display()))
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().map(|t| t.is_file()).unwrap_or(false)
                && entry.path().extension().is_some_and(|ext| ext == "md")
        })
        .count();

    assert_eq!(
        registered, on_disk,
        "plugin.json commands.length ({registered}) != top-level commands/*.md count ({on_disk})"
    );
}

#[test]
fn marketplace_json_versions_and_sources_agree() {
    let path = marketplace_json_path();
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let marketplace: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));

    let workspace_version = canonical_workspace_version();
    let workspace_root = workspace_root();
    let mut errors: Vec<String> = Vec::new();

    // metadata.version is optional; check only if present.
    if let Some(meta_version) = marketplace
        .get("metadata")
        .and_then(|m| m.get("version"))
        .and_then(|v| v.as_str())
    {
        if meta_version != workspace_version {
            errors.push(format!(
                "marketplace.json metadata.version `{meta_version}` != workspace `{workspace_version}`"
            ));
        }
    }

    let plugins = marketplace
        .get("plugins")
        .and_then(|p| p.as_array())
        .expect("marketplace.json has `plugins` array");

    assert!(
        !plugins.is_empty(),
        "marketplace.json `plugins` array is empty"
    );

    for (i, entry) in plugins.iter().enumerate() {
        let name = entry
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("<unnamed>");

        match entry.get("version").and_then(|v| v.as_str()) {
            Some(v) if v == workspace_version => {}
            Some(v) => errors.push(format!(
                "marketplace.json plugins[{i}] (`{name}`) version `{v}` != workspace `{workspace_version}`"
            )),
            None => errors.push(format!(
                "marketplace.json plugins[{i}] (`{name}`) missing string `version`"
            )),
        }

        match entry.get("source").and_then(|s| s.as_str()) {
            Some(src) => {
                let normalized = src.strip_prefix("./").unwrap_or(src);
                let abs = workspace_root.join(normalized);
                if !abs.exists() {
                    errors.push(format!(
                        "marketplace.json plugins[{i}] (`{name}`) source `{src}` does not exist on disk"
                    ));
                }
            }
            None => errors.push(format!(
                "marketplace.json plugins[{i}] (`{name}`) missing string `source`"
            )),
        }
    }

    assert!(
        errors.is_empty(),
        "marketplace.json drift detected:\n  - {}",
        errors.join("\n  - ")
    );
}
