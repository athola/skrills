//! Cold-window snapshot fixtures (TASK-006 of the cold-window plan).
//!
//! In-memory `WindowSnapshot` builders used by engine tests in
//! `skrills_analyze::cold_window::engine` (TASK-008) and downstream
//! crates. Full filesystem-tree fixtures (200 skills + 50 commands +
//! 20 plugins for SC1 / SC4 / SC5 in Sprint 4) live alongside their
//! consuming benchmarks; this module focuses on the in-memory
//! shapes the engine needs right now.

use skrills_snapshot::{
    Alert, AlertBand, HealthCheck, HealthStatus, Hint, HintCategory, LoadSample, PluginHealth,
    ResearchChannel, ResearchFinding, ScoredHint, Severity, TokenEntry, TokenLedger,
    WindowSnapshot,
};

/// Build an empty `WindowSnapshot` baseline. Useful as `prev` in
/// diff tests where only the `curr` snapshot matters.
pub fn empty_snapshot() -> WindowSnapshot {
    WindowSnapshot {
        version: 0,
        timestamp_ms: 1_700_000_000_000,
        token_ledger: TokenLedger::default(),
        alerts: vec![],
        hints: vec![],
        research_findings: vec![],
        plugin_health: vec![],
        load_sample: LoadSample::default(),
        next_tick_ms: 2_000,
    }
}

/// A canonical small fixture used by engine integration tests.
///
/// Contents:
/// - 3 skills (each ~300 tokens)
/// - 2 plugins (sums of skill totals)
/// - 1 MCP server (40K tokens — well above the Caution threshold)
/// - 1 plugin health report with one passing check
/// - Total tokens 40K + 3*300 + 2*450 = 41,800
pub fn standard_snapshot() -> WindowSnapshot {
    WindowSnapshot {
        version: 1,
        timestamp_ms: 1_700_000_000_000,
        token_ledger: TokenLedger {
            per_skill: vec![
                TokenEntry {
                    source: "skill://demo-a".into(),
                    tokens: 300,
                },
                TokenEntry {
                    source: "skill://demo-b".into(),
                    tokens: 300,
                },
                TokenEntry {
                    source: "skill://demo-c".into(),
                    tokens: 300,
                },
            ],
            per_plugin: vec![
                TokenEntry {
                    source: "plugin://alpha".into(),
                    tokens: 450,
                },
                TokenEntry {
                    source: "plugin://beta".into(),
                    tokens: 450,
                },
            ],
            per_mcp: vec![TokenEntry {
                source: "mcp://github".into(),
                tokens: 40_000,
            }],
            conversation_cache_reads: 100_000,
            conversation_cache_writes: 5_000,
            total: 41_800,
        },
        alerts: vec![],
        hints: vec![sample_scored_hint(
            sample_hint("skill://demo-a", HintCategory::Token, 5, 6.0, 4.0, 1.0),
            7.5,
            false,
        )],
        research_findings: vec![],
        plugin_health: vec![PluginHealth {
            plugin_name: "alpha".into(),
            overall: HealthStatus::Ok,
            checks: vec![HealthCheck {
                name: "manifest-parses".into(),
                status: HealthStatus::Ok,
                message: None,
            }],
        }],
        load_sample: LoadSample {
            loadavg_1min: 0.42,
            last_edit_age_ms: Some(8_000),
        },
        next_tick_ms: 2_000,
    }
}

/// Build a `LoadSample` representing a high-CPU-pressure tick.
///
/// Used by adaptive-cadence tests; satisfies the load_ratio > 0.9
/// branch in `LoadAwareCadence::next_tick` on a 4-core baseline.
pub fn high_load_sample() -> LoadSample {
    LoadSample {
        loadavg_1min: 4.0,
        last_edit_age_ms: None,
    }
}

/// Build a `LoadSample` representing an actively-edited tick.
///
/// Used by adaptive-cadence tests; satisfies the recent-edit branch.
pub fn active_edit_sample() -> LoadSample {
    LoadSample {
        loadavg_1min: 0.1,
        last_edit_age_ms: Some(3_000),
    }
}

/// Build a stream of N synthetic snapshots that exercise the alert
/// pipeline: each tick adds 5K tokens to the MCP source so the total
/// crosses Advisory → Caution → Warning → kill-switch over the
/// stream. Used by chaos-style tests.
///
/// Tick `t` has total tokens = `5_000 * t` (clamped at `u64::MAX`).
pub fn chaos_sequence(n_ticks: usize) -> Vec<WindowSnapshot> {
    (0..n_ticks)
        .map(|t| {
            let total = (t as u64).saturating_mul(5_000);
            let mcp_tokens = total.saturating_sub(1_800);
            WindowSnapshot {
                version: t as u64,
                timestamp_ms: 1_700_000_000_000 + (t as u64) * 2_000,
                token_ledger: TokenLedger {
                    per_skill: vec![
                        TokenEntry {
                            source: "skill://demo-a".into(),
                            tokens: 300,
                        },
                        TokenEntry {
                            source: "skill://demo-b".into(),
                            tokens: 300,
                        },
                        TokenEntry {
                            source: "skill://demo-c".into(),
                            tokens: 300,
                        },
                    ],
                    per_plugin: vec![
                        TokenEntry {
                            source: "plugin://alpha".into(),
                            tokens: 450,
                        },
                        TokenEntry {
                            source: "plugin://beta".into(),
                            tokens: 450,
                        },
                    ],
                    per_mcp: vec![TokenEntry {
                        source: "mcp://github".into(),
                        tokens: mcp_tokens,
                    }],
                    conversation_cache_reads: 0,
                    conversation_cache_writes: 0,
                    total,
                },
                alerts: vec![],
                hints: vec![],
                research_findings: vec![],
                plugin_health: vec![],
                load_sample: LoadSample::default(),
                next_tick_ms: 2_000,
            }
        })
        .collect()
}

/// Construct a `Hint` with explicit fields for fixture-style use.
pub fn sample_hint(
    uri: &str,
    category: HintCategory,
    frequency: u32,
    impact: f64,
    ease_score: f64,
    age_days: f64,
) -> Hint {
    Hint {
        uri: uri.to_string(),
        category,
        message: format!("synthetic hint for {uri}"),
        frequency,
        impact,
        ease_score,
        age_days,
    }
}

/// Construct a `ScoredHint` with computed score (caller-provided).
pub fn sample_scored_hint(hint: Hint, score: f64, pinned: bool) -> ScoredHint {
    ScoredHint {
        hint,
        score,
        pinned,
    }
}

/// Construct a `ResearchFinding` for fixture-style use.
pub fn sample_research_finding(
    fingerprint: &str,
    channel: ResearchChannel,
    title: &str,
    score: f64,
) -> ResearchFinding {
    ResearchFinding {
        fingerprint: fingerprint.to_string(),
        channel,
        title: title.to_string(),
        url: "https://example.com/finding".into(),
        score,
        fetched_at_ms: 1_700_000_000_000,
    }
}

/// Construct an `Alert` with explicit fields for fixture-style use.
pub fn sample_alert(
    fingerprint: &str,
    severity: Severity,
    title: &str,
    message: &str,
    band: Option<AlertBand>,
    fired_at_ms: u64,
    dwell_ticks: u32,
) -> Alert {
    Alert {
        fingerprint: fingerprint.to_string(),
        severity,
        title: title.to_string(),
        message: message.to_string(),
        band,
        fired_at_ms,
        dwell_ticks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_snapshot_total_matches_breakdown() {
        let s = standard_snapshot();
        let mcp_total: u64 = s.token_ledger.per_mcp.iter().map(|e| e.tokens).sum();
        let skill_total: u64 = s.token_ledger.per_skill.iter().map(|e| e.tokens).sum();
        let plugin_total: u64 = s.token_ledger.per_plugin.iter().map(|e| e.tokens).sum();
        assert_eq!(mcp_total + skill_total + plugin_total, s.token_ledger.total);
    }

    #[test]
    fn chaos_sequence_strictly_increases_total() {
        let seq = chaos_sequence(10);
        let totals: Vec<u64> = seq.iter().map(|s| s.token_ledger.total).collect();
        for w in totals.windows(2) {
            assert!(w[1] >= w[0], "totals should be non-decreasing: {totals:?}");
        }
    }

    #[test]
    fn chaos_sequence_first_tick_has_zero_tokens() {
        let seq = chaos_sequence(3);
        assert_eq!(seq[0].token_ledger.total, 0);
    }

    #[test]
    fn high_load_sample_exceeds_default_quad_threshold() {
        let s = high_load_sample();
        // load_ratio = 4.0 / 4_cores = 1.0 > 0.9 (HEAVY_LOAD_THRESHOLD)
        assert!(s.loadavg_1min >= 4.0);
    }

    #[test]
    fn active_edit_sample_within_recent_edit_threshold() {
        let s = active_edit_sample();
        let age = s.last_edit_age_ms.expect("edit age set");
        // RECENT_EDIT_THRESHOLD_MS = 10_000
        assert!(age < 10_000);
    }

    #[test]
    fn sample_hint_round_trips_fields() {
        let h = sample_hint("skill://x", HintCategory::Quality, 3, 7.0, 4.0, 5.0);
        assert_eq!(h.uri, "skill://x");
        assert_eq!(h.category, HintCategory::Quality);
        assert_eq!(h.frequency, 3);
    }

    #[test]
    fn empty_snapshot_is_alertable_baseline() {
        let s = empty_snapshot();
        assert_eq!(s.token_ledger.total, 0);
        assert!(s.alerts.is_empty());
    }
}
