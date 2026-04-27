//! Default alert policy for the cold-window engine.
//!
//! Implements [`AlertPolicy`] via [`LayeredAlertPolicy`] per
//! `docs/cold-window-spec.md` § 3.4 / § 5.3. Combines:
//!
//! - **4-tier classification** (Warning / Caution / Advisory / Status)
//!   from FAA AC 25.1322-1 cockpit CAS via the war-room TRIZ bridge.
//! - **Min-dwell timer**: a condition must hold for `min_dwell_ticks`
//!   consecutive ticks before the alert fires; eliminates fleeting
//!   alarms (per ISA-18.2 alarm management).
//! - **Hysteresis bands**: each alert carries `(low, low_clear, high,
//!   high_clear)`; re-arming requires re-crossing the matching
//!   `*_clear` value (tracked in [`AlertHistory`] state).
//! - **Hard kill-switch**: crossing 100% of the user-configured
//!   budget fires a `Warning` and signals the rest of the system to
//!   refuse mutating operations.
//!
//! Token thresholds (spec § 5.3, research-backed):
//!
//! - 20K total → `Advisory` (Anthropic API quadratic inflection per
//!   the Feb 2026 HN "Expensively Quadratic" analysis).
//! - 50K total → `Caution` (Willison "too many MCPs" overhead range).
//! - 80% of budget → `Warning`.
//! - 100% of budget → `Warning` + kill-switch.

use skrills_snapshot::{Alert, AlertBand, Severity, WindowSnapshot};

use super::traits::{AlertHistory, AlertPolicy};

/// Minimum dwell ticks before an alert fires (kills fleeting alarms).
pub const DEFAULT_MIN_DWELL_TICKS: u32 = 2;

/// Token total threshold for `Advisory` tier (quadratic inflection).
pub const TOKEN_ADVISORY_THRESHOLD: u64 = 20_000;

/// Token total threshold for `Caution` tier (MCP-overhead range).
pub const TOKEN_CAUTION_THRESHOLD: u64 = 50_000;

/// Fraction of budget at which a `Warning` fires (soft warning).
pub const WARNING_BUDGET_FRACTION: f64 = 0.80;

/// Hysteresis clear-band ratio: re-arm when value drops to this
/// fraction of the firing threshold.
pub const HYSTERESIS_CLEAR_RATIO: f64 = 0.95;

/// Layered alert policy with hysteresis + min-dwell + 4-tier severity.
#[derive(Debug, Clone, Copy)]
pub struct LayeredAlertPolicy {
    /// Hard token budget ceiling. Crossing this fires a `Warning`
    /// and triggers the kill-switch (the engine refuses subsequent
    /// mutating operations until the user master-acks or raises
    /// the budget).
    pub budget_ceiling: u64,
    /// How many consecutive ticks the underlying condition must
    /// hold before the alert fires.
    pub min_dwell_ticks: u32,
    /// Threshold for the `Advisory` tier (default 20K).
    pub advisory_threshold: u64,
    /// Threshold for the `Caution` tier (default 50K).
    pub caution_threshold: u64,
}

impl LayeredAlertPolicy {
    /// Construct with spec defaults; `budget_ceiling` is required
    /// (no sensible default — user-configured per `--alert-budget`).
    pub fn new(budget_ceiling: u64) -> Self {
        Self {
            budget_ceiling,
            min_dwell_ticks: DEFAULT_MIN_DWELL_TICKS,
            advisory_threshold: TOKEN_ADVISORY_THRESHOLD,
            caution_threshold: TOKEN_CAUTION_THRESHOLD,
        }
    }

    /// Override min-dwell timer.
    pub fn with_min_dwell(mut self, ticks: u32) -> Self {
        self.min_dwell_ticks = ticks;
        self
    }

    /// Override the advisory threshold.
    pub fn with_advisory_threshold(mut self, t: u64) -> Self {
        self.advisory_threshold = t;
        self
    }

    /// Override the caution threshold.
    pub fn with_caution_threshold(mut self, t: u64) -> Self {
        self.caution_threshold = t;
        self
    }

    /// Returns `true` if the current token total has breached the
    /// hard kill-switch threshold (≥100% of budget).
    pub fn kill_switch_engaged(&self, token_total: u64) -> bool {
        token_total >= self.budget_ceiling
    }

    /// Classify a token total into `(severity, band)` if alertable.
    /// Returns `None` for totals below the lowest tier threshold.
    fn classify_token_total(&self, total: u64) -> Option<(Severity, AlertBand)> {
        let budget = self.budget_ceiling as f64;
        let warning_threshold = budget * WARNING_BUDGET_FRACTION;

        // Hard kill-switch: at or above ceiling.
        if total >= self.budget_ceiling {
            return Some((
                Severity::Warning,
                AlertBand {
                    low: 0.0,
                    low_clear: 0.0,
                    high: budget,
                    high_clear: budget * HYSTERESIS_CLEAR_RATIO,
                },
            ));
        }

        // Soft warning at 80% of budget.
        if (total as f64) >= warning_threshold {
            return Some((
                Severity::Warning,
                AlertBand {
                    low: 0.0,
                    low_clear: 0.0,
                    high: warning_threshold,
                    high_clear: warning_threshold * HYSTERESIS_CLEAR_RATIO,
                },
            ));
        }

        // Caution at MCP-overhead range.
        if total >= self.caution_threshold {
            return Some((
                Severity::Caution,
                AlertBand {
                    low: 0.0,
                    low_clear: 0.0,
                    high: self.caution_threshold as f64,
                    high_clear: (self.caution_threshold as f64) * HYSTERESIS_CLEAR_RATIO,
                },
            ));
        }

        // Advisory at quadratic inflection.
        if total >= self.advisory_threshold {
            return Some((
                Severity::Advisory,
                AlertBand {
                    low: 0.0,
                    low_clear: 0.0,
                    high: self.advisory_threshold as f64,
                    high_clear: (self.advisory_threshold as f64) * HYSTERESIS_CLEAR_RATIO,
                },
            ));
        }

        None
    }

    fn title_for(severity: Severity) -> &'static str {
        match severity {
            Severity::Warning => "Token budget breach",
            Severity::Caution => "Token use entering MCP-overhead range",
            Severity::Advisory => "Token use approaching quadratic-cost inflection",
            Severity::Status => "Token status",
        }
    }

    fn fingerprint_for(severity: Severity) -> &'static str {
        match severity {
            Severity::Warning => "token-budget-warning",
            Severity::Caution => "token-budget-caution",
            Severity::Advisory => "token-budget-advisory",
            Severity::Status => "token-budget-status",
        }
    }
}

impl AlertPolicy for LayeredAlertPolicy {
    fn evaluate(
        &self,
        _prev: &WindowSnapshot,
        curr: &WindowSnapshot,
        history: &AlertHistory,
    ) -> Vec<Alert> {
        let mut alerts = Vec::new();

        if let Some((severity, band)) = self.classify_token_total(curr.token_ledger.total) {
            let fingerprint = Self::fingerprint_for(severity);
            let dwell_ticks = history
                .fingerprints
                .get(fingerprint)
                .map(|s| s.dwell_ticks + 1)
                .unwrap_or(1);

            // Min-dwell: condition must persist before firing.
            if dwell_ticks >= self.min_dwell_ticks {
                alerts.push(Alert {
                    fingerprint: fingerprint.to_string(),
                    severity,
                    title: Self::title_for(severity).to_string(),
                    message: format!(
                        "Token total: {} (ceiling: {}, advisory: {}, caution: {}).",
                        curr.token_ledger.total,
                        self.budget_ceiling,
                        self.advisory_threshold,
                        self.caution_threshold,
                    ),
                    band: Some(band),
                    fired_at_ms: curr.timestamp_ms,
                    dwell_ticks,
                });
            }
        }

        alerts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::{LoadSample, TokenLedger};

    use crate::cold_window::traits::AlertState;

    fn snapshot_with_tokens(total: u64) -> WindowSnapshot {
        WindowSnapshot {
            version: 0,
            timestamp_ms: 1_700_000_000_000,
            token_ledger: TokenLedger {
                total,
                ..Default::default()
            },
            alerts: vec![],
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        }
    }

    fn history_with_dwell(fingerprint: &str, dwell: u32) -> AlertHistory {
        let mut h = AlertHistory::new();
        h.fingerprints.insert(
            fingerprint.to_string(),
            AlertState {
                fired_at_ms: 0,
                dwell_ticks: dwell,
                cleared: false,
            },
        );
        h
    }

    #[test]
    fn below_advisory_emits_no_alert() {
        let policy = LayeredAlertPolicy::new(100_000);
        let prev = snapshot_with_tokens(0);
        let curr = snapshot_with_tokens(10_000);
        let alerts = policy.evaluate(&prev, &curr, &AlertHistory::new());
        assert!(alerts.is_empty());
    }

    #[test]
    fn advisory_threshold_with_insufficient_dwell_emits_no_alert() {
        // First-tick observation: dwell becomes 1, less than default min 2.
        let policy = LayeredAlertPolicy::new(100_000);
        let prev = snapshot_with_tokens(10_000);
        let curr = snapshot_with_tokens(25_000);
        let alerts = policy.evaluate(&prev, &curr, &AlertHistory::new());
        assert!(alerts.is_empty());
    }

    #[test]
    fn advisory_threshold_with_sufficient_dwell_fires() {
        let policy = LayeredAlertPolicy::new(100_000);
        let prev = snapshot_with_tokens(20_000);
        let curr = snapshot_with_tokens(25_000);
        let history = history_with_dwell("token-budget-advisory", 1);
        let alerts = policy.evaluate(&prev, &curr, &history);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Advisory));
    }

    #[test]
    fn caution_threshold_classifies_as_caution() {
        let policy = LayeredAlertPolicy::new(200_000);
        let curr = snapshot_with_tokens(60_000);
        let history = history_with_dwell("token-budget-caution", 1);
        let alerts = policy.evaluate(&snapshot_with_tokens(50_000), &curr, &history);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Caution));
    }

    #[test]
    fn warning_at_eighty_percent_of_budget() {
        // budget = 100K → warning threshold = 80K
        let policy = LayeredAlertPolicy::new(100_000);
        let curr = snapshot_with_tokens(85_000);
        let history = history_with_dwell("token-budget-warning", 1);
        let alerts = policy.evaluate(&snapshot_with_tokens(70_000), &curr, &history);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Warning));
    }

    #[test]
    fn warning_at_one_hundred_percent_engages_kill_switch() {
        let policy = LayeredAlertPolicy::new(100_000);
        let curr = snapshot_with_tokens(100_000);
        let history = history_with_dwell("token-budget-warning", 1);
        let alerts = policy.evaluate(&snapshot_with_tokens(95_000), &curr, &history);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Warning));
        assert!(policy.kill_switch_engaged(curr.token_ledger.total));
    }

    #[test]
    fn kill_switch_inactive_below_ceiling() {
        let policy = LayeredAlertPolicy::new(100_000);
        assert!(!policy.kill_switch_engaged(99_999));
        assert!(policy.kill_switch_engaged(100_000));
        assert!(policy.kill_switch_engaged(150_000));
    }

    #[test]
    fn min_dwell_override_changes_firing_threshold() {
        // min_dwell=1 → fires immediately on first observation
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(1);
        let prev = snapshot_with_tokens(10_000);
        let curr = snapshot_with_tokens(25_000);
        let alerts = policy.evaluate(&prev, &curr, &AlertHistory::new());
        assert_eq!(alerts.len(), 1);
    }

    #[test]
    fn alert_carries_hysteresis_band() {
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(1);
        let curr = snapshot_with_tokens(25_000);
        let alerts = policy.evaluate(&snapshot_with_tokens(0), &curr, &AlertHistory::new());
        let band = alerts[0].band.expect("band present");
        // Advisory threshold 20K, clear ratio 0.95 → high_clear = 19K
        assert_eq!(band.high, 20_000.0);
        assert!((band.high_clear - 19_000.0).abs() < 1e-9);
    }

    #[test]
    fn alert_dwell_increments_with_history() {
        let policy = LayeredAlertPolicy::new(100_000);
        let curr = snapshot_with_tokens(25_000);
        let history = history_with_dwell("token-budget-advisory", 5);
        let alerts = policy.evaluate(&snapshot_with_tokens(20_000), &curr, &history);
        assert_eq!(alerts[0].dwell_ticks, 6);
    }

    #[test]
    fn budget_ordering_warning_outranks_caution() {
        // At 80K with budget 100K, the warning band must fire (not caution),
        // because 80K is above the warning threshold AND above the caution
        // threshold. Severity classification picks the highest tier.
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(1);
        let curr = snapshot_with_tokens(80_000);
        let alerts = policy.evaluate(&snapshot_with_tokens(0), &curr, &AlertHistory::new());
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Warning));
    }

    #[test]
    fn configurable_thresholds_override_defaults() {
        let policy = LayeredAlertPolicy::new(1_000_000)
            .with_advisory_threshold(5_000)
            .with_caution_threshold(15_000)
            .with_min_dwell(1);
        let curr = snapshot_with_tokens(7_000);
        let alerts = policy.evaluate(&snapshot_with_tokens(0), &curr, &AlertHistory::new());
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Advisory));
    }
}
