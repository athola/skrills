//! Snapshot and rollback support for sync operations.
//!
//! Before each sync write phase, a snapshot is taken of the target files
//! that will be overwritten. This enables 1-command rollback via
//! `skrills sync-rollback`.
//!
//! Snapshots are stored at `~/.skrills/snapshots/<timestamp>/` by default
//! and auto-pruned after a configurable retention period (30 days).

use crate::adapters::utils::hash_content;
use crate::Result;
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;

/// Default retention period for snapshots in days.
pub const DEFAULT_RETENTION_DAYS: u64 = 30;

/// Default snapshot storage directory name under `~/.skrills/`.
const SNAPSHOTS_DIR_NAME: &str = "snapshots";

/// Manifest file name within each snapshot directory.
const MANIFEST_FILE: &str = "manifest.json";

/// Sub-directory within each snapshot directory where file backups are stored.
const FILES_DIR: &str = "files";

/// Returns the default snapshot root directory (`~/.skrills/snapshots/`).
pub fn default_snapshot_root() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Could not determine home directory")?;
    Ok(home.join(".skrills").join(SNAPSHOTS_DIR_NAME))
}

/// Configuration for snapshot behavior.
#[derive(Debug, Clone)]
pub struct SnapshotConfig {
    /// Root directory for snapshot storage.
    pub snapshot_root: PathBuf,
    /// Retention period in days. Snapshots older than this are pruned.
    pub retention_days: u64,
    /// Whether snapshots are enabled. When false, no snapshots are taken.
    pub enabled: bool,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            snapshot_root: default_snapshot_root().unwrap_or_else(|_| PathBuf::from("/tmp/skrills-snapshots")),
            retention_days: DEFAULT_RETENTION_DAYS,
            enabled: true,
        }
    }
}

/// A single file entry in a snapshot manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotEntry {
    /// Relative path from the target config root.
    pub relative_path: PathBuf,
    /// SHA256 hash of the file content before sync (empty string if file did not exist).
    pub hash_before: String,
    /// SHA256 hash of the file content after sync (empty string if unknown at snapshot time).
    pub hash_after: String,
    /// Whether the file existed before the sync.
    pub existed_before: bool,
}

/// Manifest describing a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotManifest {
    /// ISO 8601 timestamp of when the snapshot was created.
    pub timestamp: String,
    /// Unix timestamp (seconds) for easy comparison.
    pub unix_timestamp: i64,
    /// Source adapter name (e.g., "claude").
    pub source_name: String,
    /// Target adapter name (e.g., "cursor").
    pub target_name: String,
    /// Target adapter config root at time of snapshot.
    pub target_root: PathBuf,
    /// List of files captured in this snapshot.
    pub entries: Vec<SnapshotEntry>,
}

/// Summary of a snapshot for listing purposes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotSummary {
    /// Snapshot directory name (timestamp-based).
    pub id: String,
    /// ISO 8601 timestamp.
    pub timestamp: String,
    /// Source adapter name.
    pub source_name: String,
    /// Target adapter name.
    pub target_name: String,
    /// Number of files in the snapshot.
    pub file_count: usize,
    /// Full path to the snapshot directory.
    pub path: PathBuf,
}

/// Result of a restore operation.
#[derive(Debug, Clone)]
pub struct RestoreResult {
    /// Number of files restored.
    pub restored: usize,
    /// Number of files that were created by sync and deleted during rollback.
    pub deleted: usize,
    /// Warnings encountered during restore.
    pub warnings: Vec<String>,
}

/// Result of a prune operation.
#[derive(Debug, Clone)]
pub struct PruneResult {
    /// Number of snapshots pruned.
    pub pruned: usize,
    /// Number of snapshots kept.
    pub kept: usize,
}

/// Creates a snapshot of target files that will be overwritten during sync.
///
/// Reads each file in `target_paths` from disk (relative to `target_root`),
/// hashes it, and saves a backup copy. Files that don't exist yet are recorded
/// with `existed_before = false` so rollback knows to delete them.
pub fn create_snapshot(
    config: &SnapshotConfig,
    source_name: &str,
    target_name: &str,
    target_root: &Path,
    target_paths: &[PathBuf],
) -> Result<PathBuf> {
    if !config.enabled {
        bail!("Snapshots are disabled");
    }

    let now = OffsetDateTime::now_utc();
    let ts_str = now
        .format(&time::format_description::well_known::Rfc3339)
        .context("Failed to format timestamp")?;
    // Directory name: use a filesystem-safe version of the timestamp
    let dir_name = ts_str.replace(':', "-");
    let snapshot_dir = config.snapshot_root.join(&dir_name);
    let files_dir = snapshot_dir.join(FILES_DIR);

    fs::create_dir_all(&files_dir)
        .with_context(|| format!("Failed to create snapshot directory: {}", snapshot_dir.display()))?;

    let mut entries = Vec::with_capacity(target_paths.len());

    for rel_path in target_paths {
        let abs_path = target_root.join(rel_path);
        let entry = if abs_path.exists() {
            let content = fs::read(&abs_path)
                .with_context(|| format!("Failed to read target file: {}", abs_path.display()))?;
            let hash = hash_content(&content);

            // Save backup copy, preserving directory structure
            let backup_path = files_dir.join(rel_path);
            if let Some(parent) = backup_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&backup_path, &content)
                .with_context(|| format!("Failed to write backup: {}", backup_path.display()))?;

            SnapshotEntry {
                relative_path: rel_path.clone(),
                hash_before: hash,
                hash_after: String::new(),
                existed_before: true,
            }
        } else {
            SnapshotEntry {
                relative_path: rel_path.clone(),
                hash_before: String::new(),
                hash_after: String::new(),
                existed_before: false,
            }
        };
        entries.push(entry);
    }

    let manifest = SnapshotManifest {
        timestamp: ts_str,
        unix_timestamp: now.unix_timestamp(),
        source_name: source_name.to_string(),
        target_name: target_name.to_string(),
        target_root: target_root.to_path_buf(),
        entries,
    };

    let manifest_json = serde_json::to_string_pretty(&manifest)
        .context("Failed to serialize snapshot manifest")?;
    fs::write(snapshot_dir.join(MANIFEST_FILE), manifest_json)
        .context("Failed to write snapshot manifest")?;

    tracing::info!(
        snapshot = %snapshot_dir.display(),
        files = manifest.entries.len(),
        "Created sync snapshot"
    );

    Ok(snapshot_dir)
}

/// Lists all available snapshots, sorted by timestamp (newest first).
pub fn list_snapshots(config: &SnapshotConfig) -> Result<Vec<SnapshotSummary>> {
    let root = &config.snapshot_root;
    if !root.exists() {
        return Ok(vec![]);
    }

    let mut summaries = Vec::new();

    for entry in fs::read_dir(root)
        .with_context(|| format!("Failed to read snapshot directory: {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }

        match read_manifest(&manifest_path) {
            Ok(manifest) => {
                summaries.push(SnapshotSummary {
                    id: entry
                        .file_name()
                        .to_string_lossy()
                        .to_string(),
                    timestamp: manifest.timestamp.clone(),
                    source_name: manifest.source_name.clone(),
                    target_name: manifest.target_name.clone(),
                    file_count: manifest.entries.len(),
                    path: path.clone(),
                });
            }
            Err(e) => {
                tracing::warn!(
                    path = %manifest_path.display(),
                    error = %e,
                    "Skipping corrupt snapshot manifest"
                );
            }
        }
    }

    // Sort newest first by timestamp string (ISO 8601 sorts lexicographically)
    summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(summaries)
}

/// Restores the target environment from a snapshot.
///
/// If `snapshot_id` is `None`, restores the most recent snapshot.
pub fn restore_snapshot(
    config: &SnapshotConfig,
    snapshot_id: Option<&str>,
) -> Result<RestoreResult> {
    let snapshot_dir = if let Some(id) = snapshot_id {
        let dir = config.snapshot_root.join(id);
        if !dir.exists() {
            bail!("Snapshot not found: {}", id);
        }
        dir
    } else {
        // Find the most recent snapshot
        let snapshots = list_snapshots(config)?;
        if snapshots.is_empty() {
            bail!("No snapshots available to restore");
        }
        snapshots[0].path.clone()
    };

    let manifest_path = snapshot_dir.join(MANIFEST_FILE);
    let manifest = read_manifest(&manifest_path)
        .with_context(|| format!("Failed to read snapshot manifest: {}", manifest_path.display()))?;

    let files_dir = snapshot_dir.join(FILES_DIR);
    let mut restored = 0;
    let mut deleted = 0;
    let mut warnings = Vec::new();

    for entry in &manifest.entries {
        let target_path = manifest.target_root.join(&entry.relative_path);

        if entry.existed_before {
            // Restore the backup
            let backup_path = files_dir.join(&entry.relative_path);
            if backup_path.exists() {
                if let Some(parent) = target_path.parent() {
                    fs::create_dir_all(parent).ok();
                }
                match fs::copy(&backup_path, &target_path) {
                    Ok(_) => restored += 1,
                    Err(e) => {
                        warnings.push(format!(
                            "Failed to restore {}: {}",
                            entry.relative_path.display(),
                            e
                        ));
                    }
                }
            } else {
                warnings.push(format!(
                    "Backup file missing for {}",
                    entry.relative_path.display()
                ));
            }
        } else {
            // File was created by sync — remove it
            if target_path.exists() {
                match fs::remove_file(&target_path) {
                    Ok(()) => deleted += 1,
                    Err(e) => {
                        warnings.push(format!(
                            "Failed to delete {}: {}",
                            entry.relative_path.display(),
                            e
                        ));
                    }
                }
            }
        }
    }

    tracing::info!(
        snapshot = %snapshot_dir.display(),
        restored,
        deleted,
        warnings = warnings.len(),
        "Restored sync snapshot"
    );

    Ok(RestoreResult {
        restored,
        deleted,
        warnings,
    })
}

/// Prunes snapshots older than the retention period.
pub fn prune_snapshots(config: &SnapshotConfig) -> Result<PruneResult> {
    let root = &config.snapshot_root;
    if !root.exists() {
        return Ok(PruneResult {
            pruned: 0,
            kept: 0,
        });
    }

    let now = OffsetDateTime::now_utc();
    let retention_seconds = (config.retention_days * 24 * 60 * 60) as i64;

    let mut pruned = 0;
    let mut kept = 0;

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join(MANIFEST_FILE);
        if !manifest_path.exists() {
            continue;
        }

        match read_manifest(&manifest_path) {
            Ok(manifest) => {
                let age = now.unix_timestamp() - manifest.unix_timestamp;
                if age > retention_seconds {
                    match fs::remove_dir_all(&path) {
                        Ok(()) => {
                            tracing::debug!(
                                snapshot = %path.display(),
                                age_days = age / 86400,
                                "Pruned old snapshot"
                            );
                            pruned += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                snapshot = %path.display(),
                                error = %e,
                                "Failed to prune snapshot"
                            );
                            kept += 1;
                        }
                    }
                } else {
                    kept += 1;
                }
            }
            Err(_) => {
                // Corrupt manifest — prune it too
                if fs::remove_dir_all(&path).is_ok() {
                    pruned += 1;
                } else {
                    kept += 1;
                }
            }
        }
    }

    Ok(PruneResult { pruned, kept })
}

/// Collects the set of relative paths that a sync operation would write to.
///
/// This examines the target adapter's config root and the items being synced
/// to build a list of files that will be affected.
pub fn collect_target_paths(
    target_root: &Path,
    commands: &[crate::common::Command],
    skills: &[crate::common::Command],
    hooks: &[crate::common::Command],
    agents: &[crate::common::Command],
    instructions: &[crate::common::Command],
) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // For each item type, check if there's a corresponding file on the target
    for item_set in [commands, skills, hooks, agents, instructions] {
        for item in item_set {
            // The source_path is absolute — we want the file name to look for
            // on the target side. We use the name as the relative identifier.
            if let Some(file_name) = item.source_path.file_name() {
                paths.push(PathBuf::from(file_name));
            }
        }
    }

    // Deduplicate
    paths.sort();
    paths.dedup();

    paths
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_manifest(path: &Path) -> Result<SnapshotManifest> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("Failed to read manifest: {}", path.display()))?;
    let manifest: SnapshotManifest =
        serde_json::from_str(&data).context("Failed to parse snapshot manifest JSON")?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // ==========================================
    // SnapshotConfig Tests
    // ==========================================

    #[test]
    fn default_config_has_30_day_retention() {
        let config = SnapshotConfig::default();
        assert_eq!(config.retention_days, DEFAULT_RETENTION_DAYS);
        assert!(config.enabled);
    }

    // ==========================================
    // SnapshotEntry Serialization Tests
    // ==========================================

    #[test]
    fn snapshot_entry_roundtrips_through_json() {
        let entry = SnapshotEntry {
            relative_path: PathBuf::from("commands/hello.md"),
            hash_before: "abc123".to_string(),
            hash_after: "def456".to_string(),
            existed_before: true,
        };

        let json = serde_json::to_string(&entry).unwrap();
        let restored: SnapshotEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry, restored);
    }

    #[test]
    fn snapshot_manifest_roundtrips_through_json() {
        let manifest = SnapshotManifest {
            timestamp: "2026-04-13T12:00:00Z".to_string(),
            unix_timestamp: 1_776_283_200,
            source_name: "claude".to_string(),
            target_name: "cursor".to_string(),
            target_root: PathBuf::from("/home/user/.cursor"),
            entries: vec![SnapshotEntry {
                relative_path: PathBuf::from("commands/test.md"),
                hash_before: "aaa".to_string(),
                hash_after: "bbb".to_string(),
                existed_before: true,
            }],
        };

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let restored: SnapshotManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(manifest, restored);
    }

    // ==========================================
    // create_snapshot Tests
    // ==========================================

    #[test]
    fn create_snapshot_captures_existing_files() {
        let target_dir = tempdir().unwrap();
        let snapshot_root = tempdir().unwrap();

        // Create target files
        let cmds_dir = target_dir.path().join("commands");
        fs::create_dir_all(&cmds_dir).unwrap();
        fs::write(cmds_dir.join("hello.md"), "# Hello World").unwrap();
        fs::write(cmds_dir.join("greet.md"), "# Greet").unwrap();

        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        let paths = vec![
            PathBuf::from("commands/hello.md"),
            PathBuf::from("commands/greet.md"),
        ];

        let snapshot_dir =
            create_snapshot(&config, "claude", "cursor", target_dir.path(), &paths).unwrap();

        // Verify snapshot dir was created
        assert!(snapshot_dir.exists());

        // Verify manifest
        let manifest = read_manifest(&snapshot_dir.join(MANIFEST_FILE)).unwrap();
        assert_eq!(manifest.source_name, "claude");
        assert_eq!(manifest.target_name, "cursor");
        assert_eq!(manifest.entries.len(), 2);

        // All entries should have existed_before = true
        for entry in &manifest.entries {
            assert!(entry.existed_before);
            assert!(!entry.hash_before.is_empty());
        }

        // Verify backup files exist
        let files_dir = snapshot_dir.join(FILES_DIR);
        assert!(files_dir.join("commands/hello.md").exists());
        assert!(files_dir.join("commands/greet.md").exists());

        // Verify backup content matches
        let backup_content = fs::read_to_string(files_dir.join("commands/hello.md")).unwrap();
        assert_eq!(backup_content, "# Hello World");
    }

    #[test]
    fn create_snapshot_records_nonexistent_files() {
        let target_dir = tempdir().unwrap();
        let snapshot_root = tempdir().unwrap();

        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        // These files don't exist on the target
        let paths = vec![
            PathBuf::from("commands/new-skill.md"),
            PathBuf::from("commands/another.md"),
        ];

        let snapshot_dir =
            create_snapshot(&config, "claude", "codex", target_dir.path(), &paths).unwrap();

        let manifest = read_manifest(&snapshot_dir.join(MANIFEST_FILE)).unwrap();
        assert_eq!(manifest.entries.len(), 2);

        for entry in &manifest.entries {
            assert!(!entry.existed_before);
            assert!(entry.hash_before.is_empty());
        }
    }

    #[test]
    fn create_snapshot_disabled_returns_error() {
        let config = SnapshotConfig {
            snapshot_root: PathBuf::from("/tmp/unused"),
            retention_days: 30,
            enabled: false,
        };

        let result = create_snapshot(
            &config,
            "claude",
            "cursor",
            Path::new("/tmp"),
            &[PathBuf::from("file.md")],
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("disabled"));
    }

    // ==========================================
    // list_snapshots Tests
    // ==========================================

    #[test]
    fn list_snapshots_empty_dir() {
        let snapshot_root = tempdir().unwrap();
        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        let snapshots = list_snapshots(&config).unwrap();
        assert!(snapshots.is_empty());
    }

    #[test]
    fn list_snapshots_nonexistent_dir() {
        let config = SnapshotConfig {
            snapshot_root: PathBuf::from("/tmp/nonexistent-skrills-snapshots-test"),
            retention_days: 30,
            enabled: true,
        };

        let snapshots = list_snapshots(&config).unwrap();
        assert!(snapshots.is_empty());
    }

    #[test]
    fn list_snapshots_returns_sorted_newest_first() {
        let snapshot_root = tempdir().unwrap();
        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        // Create two snapshot directories with manifests
        let older_dir = snapshot_root.path().join("2026-04-10T10-00-00Z");
        let newer_dir = snapshot_root.path().join("2026-04-12T10-00-00Z");
        fs::create_dir_all(&older_dir).unwrap();
        fs::create_dir_all(&newer_dir).unwrap();

        let older_manifest = SnapshotManifest {
            timestamp: "2026-04-10T10:00:00Z".to_string(),
            unix_timestamp: 1_776_024_000,
            source_name: "claude".to_string(),
            target_name: "cursor".to_string(),
            target_root: PathBuf::from("/target"),
            entries: vec![],
        };
        let newer_manifest = SnapshotManifest {
            timestamp: "2026-04-12T10:00:00Z".to_string(),
            unix_timestamp: 1_776_196_800,
            source_name: "codex".to_string(),
            target_name: "claude".to_string(),
            target_root: PathBuf::from("/target"),
            entries: vec![SnapshotEntry {
                relative_path: PathBuf::from("file.md"),
                hash_before: "abc".to_string(),
                hash_after: String::new(),
                existed_before: true,
            }],
        };

        fs::write(
            older_dir.join(MANIFEST_FILE),
            serde_json::to_string(&older_manifest).unwrap(),
        )
        .unwrap();
        fs::write(
            newer_dir.join(MANIFEST_FILE),
            serde_json::to_string(&newer_manifest).unwrap(),
        )
        .unwrap();

        let snapshots = list_snapshots(&config).unwrap();
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[0].source_name, "codex"); // newer first
        assert_eq!(snapshots[1].source_name, "claude"); // older second
        assert_eq!(snapshots[0].file_count, 1);
        assert_eq!(snapshots[1].file_count, 0);
    }

    // ==========================================
    // restore_snapshot Tests
    // ==========================================

    #[test]
    fn restore_snapshot_restores_existing_files() {
        let target_dir = tempdir().unwrap();
        let snapshot_root = tempdir().unwrap();

        // Create original target file
        let cmds_dir = target_dir.path().join("commands");
        fs::create_dir_all(&cmds_dir).unwrap();
        fs::write(cmds_dir.join("hello.md"), "# Original Content").unwrap();

        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        // Create snapshot
        let paths = vec![PathBuf::from("commands/hello.md")];
        let snapshot_dir =
            create_snapshot(&config, "claude", "cursor", target_dir.path(), &paths).unwrap();

        // Simulate sync overwriting the file
        fs::write(cmds_dir.join("hello.md"), "# Synced Content").unwrap();
        assert_eq!(
            fs::read_to_string(cmds_dir.join("hello.md")).unwrap(),
            "# Synced Content"
        );

        // Restore
        let snapshot_id = snapshot_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let result = restore_snapshot(&config, Some(&snapshot_id)).unwrap();

        assert_eq!(result.restored, 1);
        assert_eq!(result.deleted, 0);
        assert!(result.warnings.is_empty());

        // Verify content was restored
        let restored_content = fs::read_to_string(cmds_dir.join("hello.md")).unwrap();
        assert_eq!(restored_content, "# Original Content");
    }

    #[test]
    fn restore_snapshot_deletes_new_files() {
        let target_dir = tempdir().unwrap();
        let snapshot_root = tempdir().unwrap();

        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        // Snapshot with a file that doesn't exist yet
        let paths = vec![PathBuf::from("commands/new-skill.md")];
        let snapshot_dir =
            create_snapshot(&config, "claude", "cursor", target_dir.path(), &paths).unwrap();

        // Simulate sync creating the file
        let cmds_dir = target_dir.path().join("commands");
        fs::create_dir_all(&cmds_dir).unwrap();
        fs::write(cmds_dir.join("new-skill.md"), "# New Skill").unwrap();

        // Restore
        let snapshot_id = snapshot_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let result = restore_snapshot(&config, Some(&snapshot_id)).unwrap();

        assert_eq!(result.restored, 0);
        assert_eq!(result.deleted, 1);
        assert!(result.warnings.is_empty());

        // Verify file was deleted
        assert!(!cmds_dir.join("new-skill.md").exists());
    }

    #[test]
    fn restore_snapshot_none_restores_most_recent() {
        let target_dir = tempdir().unwrap();
        let snapshot_root = tempdir().unwrap();

        // Create target file
        let cmds_dir = target_dir.path().join("commands");
        fs::create_dir_all(&cmds_dir).unwrap();
        fs::write(cmds_dir.join("hello.md"), "# Version 1").unwrap();

        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        // Create a snapshot
        let paths = vec![PathBuf::from("commands/hello.md")];
        create_snapshot(&config, "claude", "cursor", target_dir.path(), &paths).unwrap();

        // Overwrite the file
        fs::write(cmds_dir.join("hello.md"), "# Version 2").unwrap();

        // Restore most recent (None)
        let result = restore_snapshot(&config, None).unwrap();
        assert_eq!(result.restored, 1);

        let restored = fs::read_to_string(cmds_dir.join("hello.md")).unwrap();
        assert_eq!(restored, "# Version 1");
    }

    #[test]
    fn restore_snapshot_no_snapshots_returns_error() {
        let snapshot_root = tempdir().unwrap();
        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        let result = restore_snapshot(&config, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No snapshots"));
    }

    #[test]
    fn restore_snapshot_invalid_id_returns_error() {
        let snapshot_root = tempdir().unwrap();
        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        let result = restore_snapshot(&config, Some("nonexistent-snapshot"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // ==========================================
    // prune_snapshots Tests
    // ==========================================

    #[test]
    fn prune_snapshots_removes_old_snapshots() {
        let snapshot_root = tempdir().unwrap();
        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 1, // 1 day retention
            enabled: true,
        };

        // Create an "old" snapshot (90 days ago)
        let old_dir = snapshot_root.path().join("2026-01-01T00-00-00Z");
        fs::create_dir_all(old_dir.join(FILES_DIR)).unwrap();
        let old_manifest = SnapshotManifest {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            unix_timestamp: 1_767_225_600, // 2026-01-01
            source_name: "claude".to_string(),
            target_name: "cursor".to_string(),
            target_root: PathBuf::from("/target"),
            entries: vec![],
        };
        fs::write(
            old_dir.join(MANIFEST_FILE),
            serde_json::to_string(&old_manifest).unwrap(),
        )
        .unwrap();

        // Create a "recent" snapshot (now)
        let now = OffsetDateTime::now_utc();
        let recent_ts = now
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let recent_dir_name = recent_ts.replace(':', "-");
        let recent_dir = snapshot_root.path().join(&recent_dir_name);
        fs::create_dir_all(recent_dir.join(FILES_DIR)).unwrap();
        let recent_manifest = SnapshotManifest {
            timestamp: recent_ts,
            unix_timestamp: now.unix_timestamp(),
            source_name: "codex".to_string(),
            target_name: "claude".to_string(),
            target_root: PathBuf::from("/target"),
            entries: vec![],
        };
        fs::write(
            recent_dir.join(MANIFEST_FILE),
            serde_json::to_string(&recent_manifest).unwrap(),
        )
        .unwrap();

        let result = prune_snapshots(&config).unwrap();
        assert_eq!(result.pruned, 1);
        assert_eq!(result.kept, 1);

        // Old should be gone, recent should remain
        assert!(!old_dir.exists());
        assert!(recent_dir.exists());
    }

    #[test]
    fn prune_snapshots_empty_dir() {
        let snapshot_root = tempdir().unwrap();
        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        let result = prune_snapshots(&config).unwrap();
        assert_eq!(result.pruned, 0);
        assert_eq!(result.kept, 0);
    }

    #[test]
    fn prune_snapshots_nonexistent_dir() {
        let config = SnapshotConfig {
            snapshot_root: PathBuf::from("/tmp/nonexistent-skrills-prune-test"),
            retention_days: 30,
            enabled: true,
        };

        let result = prune_snapshots(&config).unwrap();
        assert_eq!(result.pruned, 0);
        assert_eq!(result.kept, 0);
    }

    // ==========================================
    // collect_target_paths Tests
    // ==========================================

    #[test]
    fn collect_target_paths_deduplicates_and_sorts() {
        use crate::common::Command;

        let cmds = vec![
            Command::new(
                "hello".into(),
                b"content".to_vec(),
                PathBuf::from("/src/hello.md"),
            ),
            Command::new(
                "greet".into(),
                b"content".to_vec(),
                PathBuf::from("/src/greet.md"),
            ),
        ];
        let skills = vec![Command::new(
            "hello".into(),
            b"content".to_vec(),
            PathBuf::from("/src/hello.md"), // same file name as commands
        )];

        let paths = collect_target_paths(
            Path::new("/target"),
            &cmds,
            &skills,
            &[],
            &[],
            &[],
        );

        // Should be deduplicated
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PathBuf::from("greet.md")));
        assert!(paths.contains(&PathBuf::from("hello.md")));
    }

    // ==========================================
    // Integration test: create → restore roundtrip
    // ==========================================

    #[test]
    fn full_snapshot_roundtrip() {
        let target_dir = tempdir().unwrap();
        let snapshot_root = tempdir().unwrap();

        // Set up target with multiple files
        let cmds_dir = target_dir.path().join("commands");
        let skills_dir = target_dir.path().join("skills");
        fs::create_dir_all(&cmds_dir).unwrap();
        fs::create_dir_all(&skills_dir).unwrap();
        fs::write(cmds_dir.join("cmd1.md"), "# Command 1").unwrap();
        fs::write(skills_dir.join("skill1.md"), "# Skill 1").unwrap();

        let config = SnapshotConfig {
            snapshot_root: snapshot_root.path().to_path_buf(),
            retention_days: 30,
            enabled: true,
        };

        let paths = vec![
            PathBuf::from("commands/cmd1.md"),
            PathBuf::from("skills/skill1.md"),
            PathBuf::from("skills/new-skill.md"), // doesn't exist yet
        ];

        // Create snapshot
        create_snapshot(&config, "claude", "cursor", target_dir.path(), &paths).unwrap();

        // Simulate sync: modify existing files and create new one
        fs::write(cmds_dir.join("cmd1.md"), "# Modified Command").unwrap();
        fs::write(skills_dir.join("skill1.md"), "# Modified Skill").unwrap();
        fs::write(skills_dir.join("new-skill.md"), "# Brand New Skill").unwrap();

        // Verify list shows one snapshot
        let snapshots = list_snapshots(&config).unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].source_name, "claude");
        assert_eq!(snapshots[0].target_name, "cursor");
        assert_eq!(snapshots[0].file_count, 3);

        // Restore
        let result = restore_snapshot(&config, None).unwrap();
        assert_eq!(result.restored, 2); // cmd1.md + skill1.md
        assert_eq!(result.deleted, 1); // new-skill.md
        assert!(result.warnings.is_empty());

        // Verify state
        assert_eq!(
            fs::read_to_string(cmds_dir.join("cmd1.md")).unwrap(),
            "# Command 1"
        );
        assert_eq!(
            fs::read_to_string(skills_dir.join("skill1.md")).unwrap(),
            "# Skill 1"
        );
        assert!(!skills_dir.join("new-skill.md").exists());
    }
}
