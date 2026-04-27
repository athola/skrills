//! Token attribution accuracy test (TASK-026).
//!
//! Asserts spec SC5: ≥95 % of tokens in a `WindowSnapshot` are
//! accounted to a specific source (skill / plugin / MCP). The
//! `TokenLedger` ground truth is its breakdown; "accuracy" is the
//! fraction of `total` that the breakdown sums to.
//!
//! Honestly fixture-bound per spec A1: this test does **not**
//! validate that token *counts* match real wire-level usage — that
//! would require running against a live Anthropic API call and is
//! out of scope for v0.8.0. What it validates is that the cold-window
//! pipeline does not lose attribution detail as snapshots flow
//! through the engine.

use skrills_analyze::cold_window::engine::TickInput;
use skrills_analyze::cold_window::ColdWindowEngine;
use skrills_snapshot::WindowSnapshot;
use skrills_test_utils::cold_window_fixtures::standard_snapshot;

/// Compute the attributed-to-source fraction of a token ledger.
///
/// Returns `total_attributed / total` clamped to `[0.0, 1.0]`.
/// `total = 0` returns `1.0` (vacuously perfect attribution).
fn attribution_ratio(snap: &WindowSnapshot) -> f64 {
    let attributed: u64 = snap
        .token_ledger
        .per_skill
        .iter()
        .map(|e| e.tokens)
        .sum::<u64>()
        + snap
            .token_ledger
            .per_plugin
            .iter()
            .map(|e| e.tokens)
            .sum::<u64>()
        + snap
            .token_ledger
            .per_mcp
            .iter()
            .map(|e| e.tokens)
            .sum::<u64>();
    if snap.token_ledger.total == 0 {
        return 1.0;
    }
    (attributed as f64 / snap.token_ledger.total as f64).min(1.0)
}

#[test]
fn standard_fixture_meets_sc5_threshold() {
    // SC5: ≥95 % attribution accuracy on the standard fixture.
    let snap = standard_snapshot();
    let ratio = attribution_ratio(&snap);
    assert!(
        ratio >= 0.95,
        "attribution ratio {ratio:.4} below SC5 floor 0.95"
    );
}

#[test]
fn tick_preserves_token_attribution() {
    // Engine tick must not drop or rewrite token entries; the
    // ledger flows through unchanged.
    let engine = ColdWindowEngine::with_defaults(100_000);
    let fixture = standard_snapshot();
    let input = TickInput::empty()
        .with_timestamp_ms(fixture.timestamp_ms)
        .with_token_ledger(fixture.token_ledger.clone());
    let snap = engine.tick(input);

    assert_eq!(
        snap.token_ledger.per_skill.len(),
        fixture.token_ledger.per_skill.len(),
        "per_skill entry count drifted across tick"
    );
    assert_eq!(
        snap.token_ledger.per_plugin.len(),
        fixture.token_ledger.per_plugin.len(),
        "per_plugin entry count drifted"
    );
    assert_eq!(
        snap.token_ledger.per_mcp.len(),
        fixture.token_ledger.per_mcp.len(),
        "per_mcp entry count drifted"
    );
    assert_eq!(snap.token_ledger.total, fixture.token_ledger.total);

    // Attribution ratio must clear SC5 after a full tick pass.
    let ratio = attribution_ratio(&snap);
    assert!(
        ratio >= 0.95,
        "post-tick attribution {ratio:.4} below SC5 floor 0.95"
    );
}

#[test]
fn empty_ledger_is_vacuously_accurate() {
    let engine = ColdWindowEngine::with_defaults(100_000);
    let snap = engine.tick(TickInput::empty());
    assert_eq!(attribution_ratio(&snap), 1.0);
}

#[test]
fn each_attributed_source_has_nonempty_uri() {
    // SC5 also implicitly requires the *named* source to be useful
    // for hint generation. A blank URI defeats user attribution.
    let snap = standard_snapshot();
    for entry in snap
        .token_ledger
        .per_skill
        .iter()
        .chain(snap.token_ledger.per_plugin.iter())
        .chain(snap.token_ledger.per_mcp.iter())
    {
        assert!(
            !entry.source.trim().is_empty(),
            "token entry has blank source"
        );
        assert!(entry.tokens > 0, "token entry has zero tokens");
    }
}

#[test]
fn attribution_ratio_handles_partial_attribution() {
    // Synthetic case: attributed sources cover 96 % of total —
    // still passes SC5 even though some tokens are unaccounted.
    use skrills_snapshot::{TokenEntry, TokenLedger};
    let mut snap = standard_snapshot();
    snap.token_ledger = TokenLedger {
        per_skill: vec![TokenEntry {
            source: "skill://x".into(),
            tokens: 96,
        }],
        per_plugin: vec![],
        per_mcp: vec![],
        conversation_cache_reads: 0,
        conversation_cache_writes: 0,
        total: 100,
    };
    let ratio = attribution_ratio(&snap);
    assert!((ratio - 0.96).abs() < 1e-9);
    assert!(ratio >= 0.95, "0.96 should clear the SC5 floor");
}

#[test]
fn attribution_ratio_below_threshold_is_detectable() {
    // Inverted assertion: a ledger with only 90 % attributed must
    // FAIL the SC5 check. This guards against the test rounding up
    // or otherwise hiding regressions.
    use skrills_snapshot::{TokenEntry, TokenLedger};
    let mut snap = standard_snapshot();
    snap.token_ledger = TokenLedger {
        per_skill: vec![TokenEntry {
            source: "skill://x".into(),
            tokens: 90,
        }],
        per_plugin: vec![],
        per_mcp: vec![],
        conversation_cache_reads: 0,
        conversation_cache_writes: 0,
        total: 100,
    };
    let ratio = attribution_ratio(&snap);
    assert!(ratio < 0.95, "0.90 must trip the SC5 floor");
}
