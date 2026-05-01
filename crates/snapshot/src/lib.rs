//! Wire-format types for the cold-window real-time analysis subsystem.
//!
//! This crate carries the contract between the cold-window producer
//! (`skrills_analyze::cold_window`) and its consumers (the TUI in
//! `skrills_dashboard` and the browser-facing SSE handler in
//! `skrills_server`). Producer and consumers depend on this crate;
//! they do not depend on each other.
//!
//! See `docs/archive/2026-04-26-cold-window-brief.md` for design rationale and
//! `docs/archive/2026-04-26-cold-window-spec.md` for type contracts. Type design rules
//! (proto-friendly conventions for the v0.9.0 gRPC follow-up) live in
//! the `types` module documentation. The `serde_impls` module
//! documents the read-tolerant deserialization strategy that reserves
//! the `{"kind": "..."}` tagged shape for future payload variants
//! without breaking the current bare-string wire format.
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

mod kill_switch;
mod serde_impls;
mod types;

pub use kill_switch::KillSwitch;
pub use types::{
    Alert, AlertBand, BandError, HealthCheck, HealthStatus, Hint, HintCategory, LoadSample,
    PluginHealth, ResearchBudget, ResearchChannel, ResearchFinding, ResearchQuota, ScoredHint,
    Severity, TokenEntry, TokenLedger, WindowSnapshot,
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
                band: Some(
                    AlertBand::new(0.0, 0.0, 80_000.0, 75_000.0).expect("fixture band valid"),
                ),
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
        let band = AlertBand::new(0.0, 5.0, 95.0, 90.0).expect("valid band");
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

    // ---------- N10: All-variants round-trip ----------

    /// Helper: serialize a value, then deserialize, then assert equality.
    fn roundtrip<T>(value: T)
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(&value).expect("serialize");
        let restored: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, value, "round-trip mismatch for {value:?}");
    }

    #[test]
    fn all_severity_variants_round_trip() {
        for v in [
            Severity::Warning,
            Severity::Caution,
            Severity::Advisory,
            Severity::Status,
        ] {
            roundtrip(v);
        }
    }

    #[test]
    fn all_hint_category_variants_round_trip() {
        for v in [
            HintCategory::Token,
            HintCategory::Validation,
            HintCategory::Redundancy,
            HintCategory::SyncDrift,
            HintCategory::Quality,
        ] {
            roundtrip(v);
        }
    }

    #[test]
    fn all_research_channel_variants_round_trip() {
        for v in [
            ResearchChannel::GitHub,
            ResearchChannel::HackerNews,
            ResearchChannel::Lobsters,
            ResearchChannel::Paper,
            ResearchChannel::Triz,
        ] {
            roundtrip(v);
        }
    }

    #[test]
    fn all_health_status_variants_round_trip() {
        for v in [
            HealthStatus::Ok,
            HealthStatus::Warn,
            HealthStatus::Error,
            HealthStatus::Unknown,
        ] {
            roundtrip(v);
        }
    }

    // ---------- N1: Read-tolerant tagged-form acceptance ----------

    #[test]
    fn severity_accepts_bare_string_form() {
        let parsed: Severity = serde_json::from_str("\"warning\"").expect("bare form");
        assert_eq!(parsed, Severity::Warning);
    }

    #[test]
    fn severity_accepts_tagged_kind_form() {
        let parsed: Severity = serde_json::from_str("{\"kind\":\"warning\"}").expect("tagged form");
        assert_eq!(parsed, Severity::Warning);
    }

    #[test]
    fn hint_category_accepts_both_forms() {
        let bare: HintCategory = serde_json::from_str("\"sync-drift\"").expect("bare");
        let tagged: HintCategory =
            serde_json::from_str("{\"kind\":\"sync-drift\"}").expect("tagged");
        assert_eq!(bare, HintCategory::SyncDrift);
        assert_eq!(tagged, HintCategory::SyncDrift);
    }

    #[test]
    fn research_channel_accepts_both_forms() {
        let bare: ResearchChannel = serde_json::from_str("\"hacker-news\"").expect("bare");
        let tagged: ResearchChannel =
            serde_json::from_str("{\"kind\":\"hacker-news\"}").expect("tagged");
        assert_eq!(bare, ResearchChannel::HackerNews);
        assert_eq!(tagged, ResearchChannel::HackerNews);
    }

    #[test]
    fn health_status_accepts_both_forms() {
        let bare: HealthStatus = serde_json::from_str("\"unknown\"").expect("bare");
        let tagged: HealthStatus = serde_json::from_str("{\"kind\":\"unknown\"}").expect("tagged");
        assert_eq!(bare, HealthStatus::Unknown);
        assert_eq!(tagged, HealthStatus::Unknown);
    }

    #[test]
    fn tagged_form_ignores_unknown_payload_fields() {
        // Forward-compat: a v0.9.0 producer may attach payload fields
        // alongside `kind`; today's consumer must not reject them.
        let parsed: Severity =
            serde_json::from_str("{\"kind\":\"caution\",\"future_field\":42}").expect("forward");
        assert_eq!(parsed, Severity::Caution);
    }

    #[test]
    fn unknown_variant_string_is_rejected() {
        let err = serde_json::from_str::<Severity>("\"bogus\"");
        assert!(err.is_err(), "unknown variant must not deserialize");
    }

    // ---------- NI4: AlertBand validation ----------

    #[test]
    fn alert_band_new_accepts_valid_thresholds() {
        let band = AlertBand::new(0.0, 5.0, 100.0, 95.0).expect("valid");
        assert_eq!(band.low(), 0.0);
        assert_eq!(band.low_clear(), 5.0);
        assert_eq!(band.high(), 100.0);
        assert_eq!(band.high_clear(), 95.0);
    }

    #[test]
    fn alert_band_new_rejects_low_greater_than_high() {
        let err = AlertBand::new(100.0, 100.0, 50.0, 50.0).expect_err("inverted");
        assert_eq!(err, BandError::MisorderedThresholds);
    }

    #[test]
    fn alert_band_new_rejects_nan() {
        for inputs in [
            (f64::NAN, 0.0, 100.0, 95.0),
            (0.0, f64::NAN, 100.0, 95.0),
            (0.0, 5.0, f64::NAN, 95.0),
            (0.0, 5.0, 100.0, f64::NAN),
        ] {
            let err = AlertBand::new(inputs.0, inputs.1, inputs.2, inputs.3)
                .expect_err("NaN must be rejected");
            assert_eq!(err, BandError::NaNValue);
        }
    }

    #[test]
    fn alert_band_new_rejects_clear_outside_band() {
        // low_clear < low
        let err = AlertBand::new(10.0, 5.0, 100.0, 95.0).expect_err("low_clear below low");
        assert_eq!(err, BandError::InvalidClear);
        // high_clear > high
        let err = AlertBand::new(0.0, 5.0, 100.0, 110.0).expect_err("high_clear above high");
        assert_eq!(err, BandError::InvalidClear);
        // low_clear > high_clear (clears overlap inverted)
        let err = AlertBand::new(0.0, 96.0, 100.0, 95.0).expect_err("clears inverted");
        assert_eq!(err, BandError::InvalidClear);
    }

    // ---------- B3: AlertBand Deserialize re-runs validation ----------
    //
    // The derived `Deserialize` (pre-B3) routed JSON straight onto
    // `pub(crate)` fields, bypassing `AlertBand::new`. A hostile or
    // v0.9.0 non-Rust producer could ship `{"low":100,"high":50,...}`
    // and downstream consumers would trust the invariants. The manual
    // impl in `serde_impls` re-runs validation; these tests pin that
    // contract.

    #[test]
    fn alert_band_deserialize_rejects_misordered_thresholds() {
        let err = serde_json::from_str::<AlertBand>(
            r#"{"low":100,"low_clear":100,"high":50,"high_clear":50}"#,
        )
        .expect_err("misordered thresholds must be rejected on the deserialize path");
        assert!(
            err.to_string().contains("low must be <= high"),
            "BandError::Display should propagate; got: {err}"
        );
    }

    #[test]
    fn alert_band_deserialize_rejects_clear_outside_band() {
        let err = serde_json::from_str::<AlertBand>(
            r#"{"low":0.0,"low_clear":-5.0,"high":100.0,"high_clear":95.0}"#,
        )
        .expect_err("low_clear below low must be rejected");
        assert!(
            err.to_string().contains("clear thresholds"),
            "BandError::InvalidClear should surface; got: {err}"
        );
    }

    #[test]
    fn alert_band_deserialize_accepts_valid_payload() {
        let band: AlertBand =
            serde_json::from_str(r#"{"low":0.0,"low_clear":5.0,"high":100.0,"high_clear":95.0}"#)
                .expect("valid band deserializes");
        assert_eq!(band.low(), 0.0);
        assert_eq!(band.high(), 100.0);
    }

    // ---------- NI8: HealthStatus default ----------

    #[test]
    fn health_status_default_is_unknown() {
        // NI8: a freshly-constructed PluginHealth must not silently
        // launder absence-of-data into "Ok". Unknown is the correct
        // sentinel.
        assert_eq!(HealthStatus::default(), HealthStatus::Unknown);
    }

    #[test]
    fn plugin_health_default_uses_unknown_overall() {
        let ph = PluginHealth::default();
        assert_eq!(ph.overall, HealthStatus::Unknown);
    }

    // ---------- NI7: ResearchQuota newtype ----------

    #[test]
    fn research_quota_new_stores_used_then_total() {
        // The constructor signature is `new(used, total)` — named
        // accessors prevent silent inversion at call sites.
        let q = ResearchQuota::new(3, 10);
        assert_eq!(q.used(), 3);
        assert_eq!(q.total(), 10);
        assert_eq!(q.available(), 7);
    }

    #[test]
    fn research_quota_available_saturates_at_zero() {
        // Defensive: if a producer reports `used > total` (clock skew,
        // refill race), `available` must not underflow into `u32::MAX`.
        let q = ResearchQuota::new(15, 10);
        assert_eq!(q.available(), 0);
    }

    #[test]
    fn research_quota_round_trips_through_json() {
        let q = ResearchQuota::new(2, 5);
        let json = serde_json::to_string(&q).expect("serialize");
        let restored: ResearchQuota = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored, q);
    }

    #[test]
    fn research_quota_inversion_is_visible_at_value_level() {
        // NI7 regression: the named-field accessors make a flipped
        // call site produce values that disagree with their meaning,
        // which any equality test catches.
        let intended = ResearchQuota::new(3, 10);
        let inverted = ResearchQuota::new(10, 3);
        assert_ne!(intended, inverted);
        assert_eq!(intended.available(), 7);
        assert_eq!(inverted.available(), 0); // saturating: 3 - 10 → 0
    }

    // ---------- S1: hoisted label / rank methods ----------

    #[test]
    fn severity_short_label_covers_every_variant() {
        assert_eq!(Severity::Warning.short_label(), "WARN");
        assert_eq!(Severity::Caution.short_label(), "CAUT");
        assert_eq!(Severity::Advisory.short_label(), "ADVI");
        assert_eq!(Severity::Status.short_label(), "STAT");
    }

    #[test]
    fn severity_rank_orders_warning_first() {
        assert!(Severity::Warning.rank() < Severity::Caution.rank());
        assert!(Severity::Caution.rank() < Severity::Advisory.rank());
        assert!(Severity::Advisory.rank() < Severity::Status.rank());
    }

    #[test]
    fn hint_category_label_covers_every_variant() {
        assert_eq!(HintCategory::Token.label(), "token");
        assert_eq!(HintCategory::Validation.label(), "validation");
        assert_eq!(HintCategory::Redundancy.label(), "redundancy");
        assert_eq!(HintCategory::SyncDrift.label(), "sync-drift");
        assert_eq!(HintCategory::Quality.label(), "quality");
    }

    #[test]
    fn research_channel_short_label_covers_every_variant() {
        assert_eq!(ResearchChannel::GitHub.short_label(), "GitHub");
        assert_eq!(ResearchChannel::HackerNews.short_label(), "HN");
        assert_eq!(ResearchChannel::Lobsters.short_label(), "Lobsters");
        assert_eq!(ResearchChannel::Paper.short_label(), "Paper");
        assert_eq!(ResearchChannel::Triz.short_label(), "TRIZ");
    }
}
