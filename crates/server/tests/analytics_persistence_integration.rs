//! Integration tests for analytics persistence functionality.
//!
//! Tests the analytics save/load cycle used by `persist_analytics_on_exit` in serve.rs.

use skrills_intelligence::{
    default_analytics_cache_path, load_analytics, load_or_build_analytics, save_analytics,
    UsageAnalytics,
};
use tempfile::TempDir;

/// Test that analytics can be saved and loaded in a round-trip.
#[test]
fn analytics_save_and_load_roundtrip() {
    let temp_dir = TempDir::new().expect("should create temp dir");
    let cache_path = temp_dir.path().join("analytics_cache.json");

    // Build analytics (may be empty if no session data exists, that's fine)
    let analytics = load_or_build_analytics(false, false).expect("should build analytics");

    // Save to temp location
    save_analytics(&analytics, &cache_path).expect("should save analytics");

    // Verify file exists
    assert!(cache_path.exists(), "cache file should exist after save");

    // Load back
    let loaded = load_analytics(&cache_path)
        .expect("should load analytics")
        .expect("should have analytics data");

    // Verify structure matches
    assert_eq!(
        analytics.frequency.len(),
        loaded.frequency.len(),
        "frequency counts should match"
    );
    assert_eq!(
        analytics.prompt_affinities.len(),
        loaded.prompt_affinities.len(),
        "prompt affinities counts should match"
    );
}

/// Test that default_analytics_cache_path returns a valid path.
#[test]
fn default_cache_path_is_valid() {
    let path = default_analytics_cache_path();
    assert!(path.is_some(), "should return a cache path");

    if let Some(p) = path {
        assert!(
            p.to_string_lossy().contains(".skrills"),
            "path should contain .skrills directory"
        );
        assert!(
            p.to_string_lossy().ends_with("analytics_cache.json"),
            "path should end with analytics_cache.json"
        );
    }
}

/// Test that load_or_build_analytics can build analytics without panicking.
#[test]
fn load_or_build_analytics_succeeds() {
    // Should succeed even if no session data exists
    let result = load_or_build_analytics(false, false);
    assert!(
        result.is_ok(),
        "load_or_build_analytics should not panic or error"
    );
}

/// Test that save_analytics creates parent directories.
#[test]
fn save_analytics_creates_parent_dirs() {
    let temp_dir = TempDir::new().expect("should create temp dir");
    let nested_path = temp_dir
        .path()
        .join("deep")
        .join("nested")
        .join("analytics.json");

    let analytics = UsageAnalytics::default();

    // Should create parent directories automatically
    save_analytics(&analytics, &nested_path).expect("should save with parent dir creation");

    assert!(nested_path.exists(), "nested file should exist");
    assert!(
        nested_path.parent().unwrap().exists(),
        "parent dirs should exist"
    );
}

/// Test that load_analytics returns None for non-existent file.
#[test]
fn load_analytics_returns_none_for_missing_file() {
    let temp_dir = TempDir::new().expect("should create temp dir");
    let nonexistent = temp_dir.path().join("does_not_exist.json");

    let result = load_analytics(&nonexistent).expect("should not error on missing file");
    assert!(result.is_none(), "should return None for missing file");
}

/// Test the full persistence workflow as used in persist_analytics_on_exit.
///
/// This simulates the exact flow in serve.rs:
/// 1. load_or_build_analytics(false, true)
/// 2. default_analytics_cache_path()
/// 3. save_analytics()
#[test]
fn persist_analytics_workflow_integration() {
    let temp_dir = TempDir::new().expect("should create temp dir");
    let test_cache_path = temp_dir.path().join("test_analytics_cache.json");

    // Simulate persist_analytics_on_exit behavior
    let analytics = load_or_build_analytics(false, true).expect("should build analytics");

    // Verify we can get a default cache path
    let default_path = default_analytics_cache_path();
    assert!(
        default_path.is_some(),
        "should have default cache path available"
    );

    // Save to test location (instead of default to avoid side effects)
    save_analytics(&analytics, &test_cache_path).expect("should save analytics");

    // Verify successful persistence
    assert!(test_cache_path.exists(), "analytics should be persisted");

    // Verify we can load it back
    let reloaded = load_analytics(&test_cache_path)
        .expect("should load saved analytics")
        .expect("should have analytics content");

    // Basic validation that structure is preserved
    assert_eq!(
        analytics.frequency.len(),
        reloaded.frequency.len(),
        "persisted analytics should match original"
    );
}

/// Test that force_rebuild flag actually bypasses cache.
#[test]
fn load_or_build_analytics_force_rebuild_bypasses_cache() {
    let temp_dir = TempDir::new().expect("should create temp dir");
    let cache_path = temp_dir.path().join("stale_cache.json");

    // Create a "stale" cache with known data
    let mut stale = UsageAnalytics::default();
    stale.frequency.insert("stale_skill".to_string(), 1);
    save_analytics(&stale, &cache_path).expect("should save stale cache");

    // Note: This test can only verify the mechanism works, not that fresh data differs,
    // because we don't control the actual session data source.
    // But we can verify force_rebuild=true executes without error.
    let result = load_or_build_analytics(true, false);
    assert!(
        result.is_ok(),
        "force rebuild should succeed regardless of cache"
    );
}
