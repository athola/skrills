//! Skill cache functionality for efficient skill discovery and metadata storage.
//!
//! This module provides an in-memory cache for discovered skills to prevent repeated
//! directory traversals. The cache includes:
//! - Time-to-live (TTL) with automatic refresh when stale
//! - Persistent snapshots for faster startup
//! - Dependency graph tracking for skill relationships
//!
//! # Concurrency Model
//!
//! The cache is designed to be shared via `Arc<Mutex<SkillCache>>` for thread-safe access.
//! This pattern is used because:
//!
//! - **Interior mutability**: The cache needs to refresh itself when stale, even during
//!   "read" operations like `skill_by_uri`. Using `Mutex` allows mutable access through
//!   a shared reference.
//!
//! - **Coarse-grained locking**: We use a single mutex around the entire cache rather
//!   than fine-grained locks because:
//!   - Most operations are fast (hash lookups, vector iteration)
//!   - Refresh operations are infrequent (TTL-gated)
//!   - Simpler reasoning about correctness
//!
//! - **`parking_lot::Mutex`** is preferred over `std::sync::Mutex` for:
//!   - No poisoning (simpler error handling)
//!   - Better performance under low contention
//!   - Smaller memory footprint
//!
//! ## Usage Pattern
//!
//! ```ignore
//! let cache: Arc<Mutex<SkillCache>> = Arc::new(Mutex::new(SkillCache::new(roots)));
//!
//! // Multiple handlers can share the cache
//! let cache_clone = Arc::clone(&cache);
//! tokio::spawn(async move {
//!     let mut guard = cache_clone.lock();
//!     let skills = guard.skills_with_dups()?;
//!     // Lock released when guard drops
//! });
//! ```
//!
//! ## Thread Safety Notes
//!
//! - All public methods that modify state require `&mut self`
//! - The snapshot path is resolved once at construction to avoid env var races
//! - File I/O is performed while holding the lock (acceptable given TTL gating)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use skrills_analyze::RelationshipGraph;
use skrills_discovery::{discover_skills, DuplicateInfo, SkillMeta, SkillRoot};
use skrills_state::{cache_ttl, home_dir, load_manifest_settings};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// An in-memory cache for discovered skills.
///
/// Stores metadata for discovered skills to prevent repeated directory traversals.
/// The cache includes a time-to-live (TTL) and automatically refreshes when stale.
///
/// # Thread Safety
///
/// This struct is **not** `Send` or `Sync` by itself. For concurrent access, wrap it
/// in `Arc<Mutex<SkillCache>>` using `parking_lot::Mutex`. See the module-level
/// documentation for the concurrency model rationale.
///
/// Most methods take `&mut self` because they may trigger a cache refresh when the
/// TTL has expired. The `_raw` suffix methods are exceptions that take `&self` but
/// should only be called after ensuring the cache is fresh via `ensure_fresh()`.
pub(crate) struct SkillCache {
    roots: Vec<SkillRoot>,
    ttl: Duration,
    last_scan: Option<Instant>,
    skills: Vec<SkillMeta>,
    duplicates: Vec<DuplicateInfo>,
    uri_index: HashMap<String, usize>,
    /// Snapshot path is resolved once to avoid cross-test/env races
    snapshot_path: Option<PathBuf>,
    /// Relationship graph for skill dependencies (simple graph, not full resolver)
    dep_graph: RelationshipGraph,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SkillCacheSnapshot {
    roots: Vec<String>,
    last_scan: u64,
    skills: Vec<SkillMeta>,
    duplicates: Vec<DuplicateInfo>,
}

impl SkillCache {
    /// Create a new `SkillCache` with the given roots.
    #[allow(dead_code)]
    pub(crate) fn new(roots: Vec<SkillRoot>) -> Self {
        Self::new_with_ttl(roots, cache_ttl(&load_manifest_settings))
    }

    /// Create a new `SkillCache` with the given roots and TTL.
    pub(crate) fn new_with_ttl(roots: Vec<SkillRoot>, ttl: Duration) -> Self {
        let snapshot_path = Self::resolve_snapshot_path();
        let mut cache = Self {
            roots,
            ttl,
            last_scan: None,
            skills: Vec::new(),
            duplicates: Vec::new(),
            uri_index: HashMap::new(),
            snapshot_path,
            dep_graph: RelationshipGraph::new(),
        };
        if let Err(e) = cache.try_load_snapshot() {
            tracing::debug!(
                target: "skrills::startup",
                error = %e,
                "failed to load discovery snapshot; will rescan"
            );
        }
        cache
    }

    #[allow(dead_code)]
    pub(crate) fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Returns the paths of the root directories being watched.
    pub(crate) fn watched_roots(&self) -> Vec<PathBuf> {
        self.roots.iter().map(|r| r.root.clone()).collect()
    }

    /// Resolve snapshot path once to prevent later env churn from redirecting cache IO.
    fn resolve_snapshot_path() -> Option<PathBuf> {
        if let Ok(path) = std::env::var("SKRILLS_CACHE_PATH") {
            return Some(PathBuf::from(path));
        }
        match home_dir() {
            Ok(h) => Some(h.join(".codex/skills-cache.json")),
            Err(e) => {
                tracing::debug!(target: "skrills::startup", error=%e, "could not resolve home dir for snapshot");
                None
            }
        }
    }

    fn snapshot_path(&self) -> Option<PathBuf> {
        self.snapshot_path.clone()
    }

    fn roots_fingerprint(&self) -> Vec<String> {
        self.roots
            .iter()
            .map(|r| r.root.to_string_lossy().into_owned())
            .collect()
    }

    /// Build the dependency graph for a set of skills.
    fn build_dependency_graph(&self, skills: &[SkillMeta]) -> RelationshipGraph {
        let mut dep_graph = RelationshipGraph::new();
        for skill in skills {
            let skill_uri = format!("skill://skrills/{}/{}", skill.source.label(), skill.name);
            dep_graph.add_skill(&skill_uri);

            // Analyze dependencies
            if let Ok(content) = fs::read_to_string(&skill.path) {
                let analysis = skrills_analyze::analyze_dependencies(&skill.path, &content);

                tracing::debug!(
                    target: "skrills::deps",
                    skill = %skill.name,
                    total_deps = analysis.dependencies.len(),
                    "analyzing dependencies"
                );

                // Extract skill dependencies and convert to URIs
                for dep in &analysis.dependencies {
                    tracing::debug!(
                        target: "skrills::deps",
                        skill = %skill.name,
                        dep_type = ?dep.dep_type,
                        dep_target = %dep.target,
                        "found dependency"
                    );

                    if dep.dep_type == skrills_analyze::DependencyType::Skill {
                        // Try to resolve the dependency path to a skill URI
                        if let Some(dep_uri) =
                            self.resolve_dependency_to_uri(&skill.path, &dep.target, skills)
                        {
                            tracing::debug!(
                                target: "skrills::deps",
                                skill = %skill.name,
                                dependency = %dep_uri,
                                "added dependency"
                            );
                            dep_graph.add_dependency(&skill_uri, &dep_uri);
                        } else {
                            tracing::debug!(
                                target: "skrills::deps",
                                skill = %skill.name,
                                dep_path = %dep.target,
                                "failed to resolve dependency"
                            );
                        }
                    }
                }
            }
        }
        dep_graph
    }

    /// Resolve a dependency path to a skill URI.
    ///
    /// Takes a relative path from a skill file and tries to find the corresponding skill.
    fn resolve_dependency_to_uri(
        &self,
        skill_path: &Path,
        dep_path: &str,
        skills: &[SkillMeta],
    ) -> Option<String> {
        // Get the directory containing the skill
        let skill_dir = skill_path.parent()?;

        // Resolve the dependency path relative to the skill directory
        let resolved_path = skill_dir.join(dep_path);
        let canonical_path = resolved_path.canonicalize().ok()?;

        // Find the skill that matches this path
        for skill in skills {
            if let Ok(skill_canonical) = skill.path.canonicalize() {
                if skill_canonical == canonical_path {
                    return Some(format!(
                        "skill://skrills/{}/{}",
                        skill.source.label(),
                        skill.name
                    ));
                }
            }
        }

        None
    }

    /// Attempt to load a persisted snapshot if it is still within TTL and roots match.
    fn try_load_snapshot(&mut self) -> Result<()> {
        let Some(path) = self.snapshot_path() else {
            return Ok(());
        };
        if !path.exists() {
            return Ok(());
        }
        let data = fs::read_to_string(&path)?;
        let snap: SkillCacheSnapshot = serde_json::from_str(&data)?;

        let current_roots = self.roots_fingerprint();
        if snap.roots != current_roots {
            tracing::warn!(
                target: "skrills::startup",
                "snapshot roots mismatch: expected {:?}, got {:?}",
                current_roots,
                snap.roots
            );
            return Ok(());
        }

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let age = now_secs.saturating_sub(snap.last_scan);
        if age as u128 > self.ttl.as_secs() as u128 {
            tracing::warn!(
                target: "skrills::startup",
                "snapshot stale: age {}s > ttl {}s",
                age,
                self.ttl.as_secs()
            );
            return Ok(());
        }

        let mut uri_index = HashMap::new();
        for (idx, s) in snap.skills.iter().enumerate() {
            // New canonical URI with server "skrills" and source in path.
            uri_index.insert(
                format!("skill://skrills/{}/{}", s.source.label(), s.name),
                idx,
            );
            // Backward-compatible legacy URI (no server component).
            uri_index.insert(format!("skill://{}/{}", s.source.label(), s.name), idx);
        }
        self.skills = snap.skills.clone();
        self.duplicates = snap.duplicates;
        self.uri_index = uri_index;

        // Build dependency graph for loaded skills
        self.dep_graph = self.build_dependency_graph(&snap.skills);

        self.last_scan = Some(Instant::now());
        tracing::info!(
            target: "skrills::startup",
            skills = self.skills.len(),
            "loaded discovery snapshot"
        );
        Ok(())
    }

    fn persist_snapshot(&self) {
        if let Some(path) = self.snapshot_path() {
            let snap = SkillCacheSnapshot {
                roots: self.roots_fingerprint(),
                last_scan: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
                skills: self.skills.clone(),
                duplicates: self.duplicates.clone(),
            };
            if let Ok(text) = serde_json::to_string(&snap) {
                if let Err(e) = fs::write(&path, text) {
                    tracing::debug!(target: "skrills::startup", error=%e, "failed to persist snapshot");
                }
            }
        }
    }

    /// Invalidate the cache, forcing a rescan on the next access.
    pub(crate) fn invalidate(&mut self) {
        self.last_scan = None;
        self.skills.clear();
        self.duplicates.clear();
        self.uri_index.clear();
        self.dep_graph = RelationshipGraph::new();
    }

    /// Refresh the cache if the TTL has expired or the cache is empty.
    fn refresh_if_stale(&mut self) -> Result<()> {
        let now = Instant::now();
        let fresh = self
            .last_scan
            .map(|ts| now.duration_since(ts) < self.ttl)
            .unwrap_or(false);
        if fresh {
            return Ok(());
        }

        // If we've been invalidated (or never loaded) attempt a cheap snapshot reload.
        // When a snapshot exists, serve it immediately to avoid dropping cached skills
        // if the filesystem scan comes back empty (e.g. transiently missing paths).
        if self.last_scan.is_none() && self.skills.is_empty() {
            if let Err(e) = self.try_load_snapshot() {
                tracing::debug!(
                    target: "skrills::startup",
                    error = %e,
                    "failed to reload discovery snapshot after invalidation"
                );
            } else if !self.skills.is_empty() {
                // Serve the snapshot immediately; schedule a rescan on the next access by
                // backdating `last_scan` so it appears stale after this return.
                let backdate = self
                    .ttl
                    .checked_add(Duration::from_millis(1))
                    .unwrap_or(self.ttl);
                self.last_scan = Some(now.checked_sub(backdate).unwrap_or(now));
                return Ok(());
            }
        }

        let scan_started = Instant::now();
        let mut dup_log = Vec::new();
        let skills = discover_skills(&self.roots, Some(&mut dup_log))?;
        let mut uri_index = HashMap::new();
        for (idx, s) in skills.iter().enumerate() {
            // New canonical URI with server "skrills" and source in path.
            uri_index.insert(
                format!("skill://skrills/{}/{}", s.source.label(), s.name),
                idx,
            );
            // Backward-compatible legacy URI (no server component).
            uri_index.insert(format!("skill://{}/{}", s.source.label(), s.name), idx);
        }

        // Build dependency graph
        let dep_graph = self.build_dependency_graph(&skills);

        self.skills = skills;
        self.duplicates = dup_log;
        self.uri_index = uri_index;
        self.dep_graph = dep_graph;
        self.last_scan = Some(now);
        self.persist_snapshot();
        let elapsed_ms = scan_started.elapsed().as_millis();
        if elapsed_ms > 250 {
            tracing::info!(
                target: "skrills::scan",
                elapsed_ms,
                roots = self.roots.len(),
                skills = self.skills.len(),
                "skill discovery completed"
            );
        } else {
            tracing::debug!(
                target: "skrills::scan",
                elapsed_ms,
                roots = self.roots.len(),
                skills = self.skills.len(),
                "skill discovery completed"
            );
        }
        Ok(())
    }

    /// Returns the current list of skills and any recorded duplicate information.
    pub(crate) fn skills_with_dups(&mut self) -> Result<(Vec<SkillMeta>, Vec<DuplicateInfo>)> {
        self.refresh_if_stale()?;
        Ok((self.skills.clone(), self.duplicates.clone()))
    }

    /// Retrieve a skill by its URI.
    pub(crate) fn skill_by_uri(&mut self, uri: &str) -> Result<SkillMeta> {
        self.refresh_if_stale()?;
        if let Some(idx) = self.uri_index.get(uri).copied() {
            return Ok(self.skills[idx].clone());
        }
        Err(anyhow::anyhow!("skill not found"))
    }

    /// Get transitive dependencies for a skill URI.
    pub(crate) fn resolve_dependencies(&mut self, uri: &str) -> Result<Vec<String>> {
        self.refresh_if_stale()?;
        Ok(self.dep_graph.resolve(uri))
    }

    /// Get direct (non-transitive) dependencies for a skill URI.
    pub(crate) fn get_direct_dependencies(&mut self, uri: &str) -> Result<Vec<String>> {
        self.refresh_if_stale()?;
        let deps = self.dep_graph.dependencies(uri);
        let mut result: Vec<String> = deps.into_iter().collect();
        result.sort();
        Ok(result)
    }

    /// Get all skill URIs in the dependency graph.
    pub(crate) fn skill_uris(&mut self) -> Result<Vec<String>> {
        self.refresh_if_stale()?;
        Ok(self.dep_graph.skills())
    }

    /// Get direct dependencies for a skill URI (raw access without refresh).
    ///
    /// This is used for computing dependency statistics and should only be called
    /// after ensuring the cache is fresh.
    pub(crate) fn dependencies_raw(&self, uri: &str) -> Vec<String> {
        self.dep_graph.dependencies(uri).into_iter().collect()
    }

    /// Get direct dependents for a skill URI (raw access without refresh).
    ///
    /// This is used for computing dependency statistics and should only be called
    /// after ensuring the cache is fresh.
    pub(crate) fn dependents_raw(&self, uri: &str) -> Vec<String> {
        self.dep_graph.dependents(uri)
    }

    /// Ensure the cache is refreshed if stale.
    pub(crate) fn ensure_fresh(&mut self) -> Result<()> {
        self.refresh_if_stale()
    }

    /// Get skills that depend on the given skill URI.
    pub(crate) fn get_dependents(&mut self, uri: &str) -> Result<Vec<String>> {
        self.refresh_if_stale()?;
        Ok(self.dep_graph.dependents(uri))
    }

    /// Get all skills that transitively depend on the given skill URI.
    pub(crate) fn get_transitive_dependents(&mut self, uri: &str) -> Result<Vec<String>> {
        self.refresh_if_stale()?;
        Ok(self.dep_graph.transitive_dependents(uri))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support;
    use skrills_discovery::SkillSource;
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.previous {
                std::env::set_var(self.key, v);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = std::env::var(key).ok();
        if let Some(val) = value {
            std::env::set_var(key, val);
        } else {
            std::env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    fn write_skill(root: &Path, name: &str) -> PathBuf {
        let skill_dir = root.join(name);
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        let path = skill_dir.join("SKILL.md");
        fs::write(&path, "demo skill").expect("write skill");
        path
    }

    #[test]
    fn cache_refreshes_after_ttl_and_discovers_new_skill() {
        /*
        GIVEN a cache with a short TTL
        WHEN new skills appear after the TTL
        THEN refresh should discover them
        */
        let _guard = test_support::env_guard();
        let temp = tempdir().expect("tempdir");
        let cache_path = temp.path().join("skills-cache.json");
        let _cache_env = set_env_var(
            "SKRILLS_CACHE_PATH",
            Some(cache_path.to_str().expect("cache path")),
        );

        let root_dir = temp.path().join("skills");
        write_skill(&root_dir, "alpha");
        let root = SkillRoot {
            root: root_dir.clone(),
            source: SkillSource::Codex,
        };

        let mut cache = SkillCache::new_with_ttl(vec![root], Duration::from_millis(5));
        let (skills, _dups) = cache
            .skills_with_dups()
            .expect("initial skill scan should succeed");
        assert!(
            skills.iter().any(|s| s.name.contains("alpha")),
            "alpha should be present"
        );

        write_skill(&root_dir, "beta");

        let started = Instant::now();
        loop {
            let (skills, _dups) = cache.skills_with_dups().expect("refresh should succeed");
            if skills.len() >= 2 {
                break;
            }
            if started.elapsed() >= Duration::from_secs(1) {
                panic!("cache did not refresh within expected time");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn cache_invalidate_triggers_rescan_after_snapshot() {
        /*
        GIVEN a cache with an existing snapshot
        WHEN invalidate is called and the underlying skill is removed
        THEN a subsequent refresh should reflect the removal
        */
        let _guard = test_support::env_guard();
        let temp = tempdir().expect("tempdir");
        let cache_path = temp.path().join("skills-cache.json");
        let _cache_env = set_env_var(
            "SKRILLS_CACHE_PATH",
            Some(cache_path.to_str().expect("cache path")),
        );

        let root_dir = temp.path().join("skills");
        let skill_path = write_skill(&root_dir, "alpha");
        let root = SkillRoot {
            root: root_dir.clone(),
            source: SkillSource::Codex,
        };

        let mut cache = SkillCache::new_with_ttl(vec![root], Duration::from_secs(60));
        let (skills, _dups) = cache
            .skills_with_dups()
            .expect("initial skill scan should succeed");
        assert_eq!(skills.len(), 1, "expected one skill in cache");

        fs::remove_file(&skill_path).expect("remove skill");
        cache.invalidate();

        let _ = cache
            .skills_with_dups()
            .expect("snapshot reload should succeed");
        let (skills, _dups) = cache.skills_with_dups().expect("rescan should succeed");
        assert!(skills.is_empty(), "expected cache to drop removed skill");
    }

    #[test]
    fn cache_supports_parallel_reads() {
        /*
        GIVEN a shared cache behind a mutex
        WHEN multiple threads read concurrently
        THEN all reads should succeed without panic
        */
        let _guard = test_support::env_guard();
        let temp = tempdir().expect("tempdir");
        let cache_path = temp.path().join("skills-cache.json");
        let _cache_env = set_env_var(
            "SKRILLS_CACHE_PATH",
            Some(cache_path.to_str().expect("cache path")),
        );

        let root_dir = temp.path().join("skills");
        write_skill(&root_dir, "alpha");
        let root = SkillRoot {
            root: root_dir.clone(),
            source: SkillSource::Codex,
        };

        let cache = SkillCache::new_with_ttl(vec![root], Duration::from_secs(60));
        let shared = Arc::new(parking_lot::Mutex::new(cache));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let shared = Arc::clone(&shared);
                std::thread::spawn(move || {
                    let mut guard = shared.lock();
                    let (skills, _dups) =
                        guard.skills_with_dups().expect("cache read should succeed");
                    assert!(!skills.is_empty(), "expected skills in cache");
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("thread join");
        }
    }
}
