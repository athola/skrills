//! Sync tests - sync error paths and parameter handling

use super::super::*;
use crate::cli::SyncSource;
use std::time::Duration;

/// T1: run_sync_with_adapters rejects same source and target.
///
/// Given: from == to (both Claude)
/// When: run_sync_with_adapters is called
/// Then: It returns an error mentioning "cannot be the same"
#[test]
fn run_sync_with_adapters_same_source_returns_error() {
    let params = skrills_sync::SyncParams {
        dry_run: true,
        ..Default::default()
    };

    let result = run_sync_with_adapters(SyncSource::Claude, SyncSource::Claude, &params);
    assert!(result.is_err(), "Expected error when from == to");
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("cannot be the same"),
        "Error should mention 'cannot be the same', got: {msg}"
    );
}

/// T1 additional: verify the same-source error for all SyncSource variants.
#[test]
fn run_sync_with_adapters_same_source_all_variants() {
    let params = skrills_sync::SyncParams {
        dry_run: true,
        ..Default::default()
    };

    for source in [
        SyncSource::Claude,
        SyncSource::Codex,
        SyncSource::Copilot,
        SyncSource::Cursor,
    ] {
        let result = run_sync_with_adapters(source, source, &params);
        assert!(
            result.is_err(),
            "Expected error for {:?} -> {:?}",
            source,
            source
        );
    }
}

/// T2: sync_skills_tool uses explicit `to` parameter instead of the default.
///
/// Given: from = "claude", to = "claude" (explicitly set, overriding default "codex")
/// When: sync_skills_tool is called
/// Then: The explicit `to` value is used, triggering the same-source error
///        (proving the explicit value was used rather than the default "codex")
#[test]
fn sync_skills_tool_explicit_to_overrides_default() {
    let service =
        SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).expect("create service");
    let mut args = serde_json::Map::new();
    args.insert("from".into(), serde_json::json!("claude"));
    args.insert("to".into(), serde_json::json!("claude"));
    args.insert("dry_run".into(), serde_json::json!(true));

    let result = service.sync_skills_tool(args);
    assert!(
        result.is_err(),
        "Expected error because explicit to=claude overrides default to=codex"
    );
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("cannot be the same"),
        "Error should mention 'cannot be the same', got: {msg}"
    );
}

/// Default sync target is applied when no explicit `to` is provided.
///
/// Given: from = "claude", no `to` parameter
/// When: sync_skills_tool is called
/// Then: The default target "codex" is used (no same-source error)
#[test]
fn sync_skills_tool_uses_default_target_when_to_omitted() {
    let service =
        SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).expect("create service");
    let mut args = serde_json::Map::new();
    args.insert("from".into(), serde_json::json!("claude"));
    args.insert("dry_run".into(), serde_json::json!(true));
    // No "to" parameter — should default to "codex" per default_target_for("claude")

    let result = service.sync_skills_tool(args);
    // Should NOT error with "cannot be the same" since default target differs from source
    assert!(
        result.is_ok(),
        "Expected success when using default target, got: {:?}",
        result.err()
    );
}
