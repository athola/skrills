//! Wire-format types for the cold-window real-time analysis subsystem.
//!
//! This crate carries the contract between the cold-window producer
//! (`skrills_analyze::cold_window`) and its consumers (the TUI in
//! `skrills_dashboard` and the browser-facing SSE handler in
//! `skrills_server`). Producer and consumers depend on this crate;
//! they do not depend on each other.
//!
//! See `docs/cold-window-brief.md` for design rationale and
//! `docs/cold-window-spec.md` for type contracts. Type design rules
//! (proto-friendly conventions for the v0.9.0 gRPC follow-up) live in
//! the `types` module documentation.
//!
//! # Example
//!
//! ```
//! use skrills_snapshot::{WindowSnapshot, TokenLedger, LoadSample};
//!
//! let snap = WindowSnapshot {
//!     version: 1,
//!     timestamp_ms: 1_700_000_000_000,
//!     token_ledger: TokenLedger::default(),
//!     alerts: vec![],
//!     hints: vec![],
//!     research_findings: vec![],
//!     plugin_health: vec![],
//!     load_sample: LoadSample::default(),
//!     next_tick_ms: 2_000,
//! };
//! assert_eq!(snap.version, 1);
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod types;

pub use types::{
    Alert, AlertBand, HealthCheck, HealthStatus, Hint, HintCategory, LoadSample, PluginHealth,
    ResearchBudget, ResearchChannel, ResearchFinding, ScoredHint, Severity, TokenEntry,
    TokenLedger, WindowSnapshot,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> WindowSnapshot {
        WindowSnapshot {
            version: 42,
            timestamp_ms: 1_700_000_000_000,
            token_ledger: TokenLedger {
                per_skill: vec![TokenEntry {
                    source: "skill://demo".into(),
                    tokens: 1234,
                }],
                per_plugin: vec![],
                per_mcp: vec![TokenEntry {
                    source: "mcp://github".into(),
                    tokens: 55_000,
                }],
                conversation_cache_reads: 1_000_000,
                conversation_cache_writes: 50_000,
                total: 1_106_234,
            },
            alerts: vec![Alert {
                fingerprint: "token-budget-80".into(),
                severity: Severity::Warning,
                title: "Token budget at 80%".into(),
                message: "MCP tool descriptions dominate.".into(),
                band: Some(AlertBand {
                    low: 0.0,
                    low_clear: 0.0,
                    high: 80_000.0,
                    high_clear: 75_000.0,
                }),
                fired_at_ms: 1_700_000_000_000,
                dwell_ticks: 3,
            }],
            hints: vec![ScoredHint {
                hint: Hint {
                    uri: "skill://demo".into(),
                    category: HintCategory::Redundancy,
                    message: "Two skills cover the same capability.".into(),
                    frequency: 5,
                    impact: 4.5,
                    ease_score: 6.0,
                    age_days: 2.0,
                },
                score: 7.83,
                pinned: false,
            }],
            research_findings: vec![ResearchFinding {
                fingerprint: "token-budget-80".into(),
                channel: ResearchChannel::HackerNews,
                title: "Expensively Quadratic LLM Cost Curve".into(),
                url: "https://news.ycombinator.com/item?id=47000034".into(),
                score: 142.0,
                fetched_at_ms: 1_700_000_000_000,
            }],
            plugin_health: vec![PluginHealth {
                plugin_name: "skrills".into(),
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

    #[test]
    fn snapshot_round_trips_through_json() {
        let original = fixture();
        let serialized = serde_json::to_string(&original).expect("serialize");
        let restored: WindowSnapshot = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(restored, original);
    }

    #[test]
    fn severity_serializes_lowercase() {
        let json = serde_json::to_string(&Severity::Warning).expect("serialize");
        assert_eq!(json, "\"warning\"");
    }

    #[test]
    fn hint_category_uses_kebab_case() {
        let json = serde_json::to_string(&HintCategory::SyncDrift).expect("serialize");
        assert_eq!(json, "\"sync-drift\"");
    }

    #[test]
    fn research_channel_uses_kebab_case() {
        let json = serde_json::to_string(&ResearchChannel::HackerNews).expect("serialize");
        assert_eq!(json, "\"hacker-news\"");
    }

    #[test]
    fn cadence_label_uses_active_edit_when_recent() {
        let mut snap = fixture();
        snap.next_tick_ms = 2_000;
        snap.load_sample.last_edit_age_ms = Some(8_000);
        snap.load_sample.loadavg_1min = 0.0;
        assert_eq!(snap.cadence_label(), "tick: 2.0s [active edit]");
    }

    #[test]
    fn cadence_label_uses_load_when_above_zero() {
        let mut snap = fixture();
        snap.next_tick_ms = 1_500;
        snap.load_sample.last_edit_age_ms = None;
        snap.load_sample.loadavg_1min = 0.42;
        assert_eq!(snap.cadence_label(), "tick: 1.5s [load 0.42]");
    }

    #[test]
    fn cadence_label_falls_back_to_base() {
        let mut snap = fixture();
        snap.next_tick_ms = 2_000;
        snap.load_sample.last_edit_age_ms = None;
        snap.load_sample.loadavg_1min = 0.0;
        assert_eq!(snap.cadence_label(), "tick: 2.0s [base]");
    }

    #[test]
    fn alert_band_round_trips() {
        let band = AlertBand {
            low: 0.0,
            low_clear: 5.0,
            high: 95.0,
            high_clear: 90.0,
        };
        let json = serde_json::to_string(&band).expect("serialize");
        let restored: AlertBand = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, band);
    }

    #[test]
    fn token_ledger_default_is_empty() {
        let ledger = TokenLedger::default();
        assert_eq!(ledger.total, 0);
        assert!(ledger.per_skill.is_empty());
    }

    #[test]
    fn load_sample_optional_edit_serializes_as_null() {
        let sample = LoadSample {
            loadavg_1min: 0.5,
            last_edit_age_ms: None,
        };
        let json = serde_json::to_string(&sample).expect("serialize");
        assert!(json.contains("\"last_edit_age_ms\":null"));
    }
}
