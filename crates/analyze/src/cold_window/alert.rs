//! Default alert policy for the cold-window engine.
//!
//! Implements [`AlertPolicy`] via [`LayeredAlertPolicy`] per
//! `docs/archive/2026-04-26-cold-window-spec.md` § 3.4 / § 5.3. Combines:
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

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use skrills_snapshot::{Alert, AlertBand, Severity, WindowSnapshot};

use super::traits::{AlertHistory, AlertPolicy, AlertState};

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

/// Number of samples the rolling baseline window holds before it
/// switches from static thresholds to adaptive (mean ± k·σ) thresholds.
/// Sized for ~1 minute of activity at 1 Hz tick — small enough for
/// responsive adaptation, large enough that early jitter does not
/// dominate the sample mean. Spec § 4.3 (NI1 SC9 rolling baseline).
pub const MIN_BASELINE_SAMPLES: usize = 60;

/// Hard cap on the rolling baseline window. Spec § 4.3 names "5
/// minutes" as the target window; at the slowest cadence (8 s tick)
/// that is ~37 samples, at the fastest cadence (500 ms tick) it is
/// 600 samples. We pick 600 as the upper bound so the window never
/// grows unbounded.
pub const MAX_BASELINE_SAMPLES: usize = 600;

/// Adaptive threshold sigma multiplier for the **Advisory** tier.
pub const ADVISORY_SIGMA_K: f64 = 2.0;

/// Adaptive threshold sigma multiplier for the **Caution** tier.
pub const CAUTION_SIGMA_K: f64 = 3.0;

/// Adaptive threshold sigma multiplier for the **Warning** tier.
/// (Used only for the rolling-baseline floor; the absolute warning
/// threshold remains anchored at 80 % of the user's budget ceiling.)
pub const WARNING_SIGMA_K: f64 = 4.0;

/// Validation failure when constructing or mutating a
/// [`LayeredAlertPolicy`] (NI6 builder validation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    /// Tier ordering violated:
    /// `advisory_threshold < caution_threshold < budget_ceiling`.
    MisorderedTiers,
    /// `min_dwell_ticks` was zero (would render dwell semantics
    /// vacuous — every observation would fire immediately).
    ZeroDwell,
}

impl core::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::MisorderedTiers => write!(
                f,
                "LayeredAlertPolicy: tiers must satisfy advisory < caution < budget_ceiling"
            ),
            Self::ZeroDwell => write!(f, "LayeredAlertPolicy: min_dwell_ticks must be >= 1"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// Rolling baseline window of token-total samples (NI1 SC9).
///
/// Maintained internally by [`LayeredAlertPolicy`]; a fresh window is
/// created per policy instance. We compute mean + sample-stddev on
/// demand at evaluate-time; the per-tick cost is `O(N)` with `N` ≤
/// [`MAX_BASELINE_SAMPLES`], which at the engine's tight tick budget
/// (~50 ms) is comfortably under the SC1 budget for any realistic N.
///
/// Pre-warmup behavior (size < [`MIN_BASELINE_SAMPLES`]): no
/// adaptation — the policy's static thresholds drive classification.
#[derive(Debug, Default)]
pub struct BaselineWindow {
    samples: VecDeque<u64>,
    capacity: usize,
}

impl BaselineWindow {
    /// Construct with the spec default capacity ([`MAX_BASELINE_SAMPLES`]).
    pub fn new() -> Self {
        Self::with_capacity(MAX_BASELINE_SAMPLES)
    }

    /// Construct with a user-supplied capacity (clamped to
    /// `[MIN_BASELINE_SAMPLES, MAX_BASELINE_SAMPLES]`).
    pub fn with_capacity(capacity: usize) -> Self {
        let capacity = capacity.clamp(MIN_BASELINE_SAMPLES, MAX_BASELINE_SAMPLES);
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Append a sample, evicting the oldest if at capacity.
    pub fn push(&mut self, sample: u64) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    /// Number of samples currently in the window.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// True when the window holds no samples.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// True when the window has not yet reached
    /// [`MIN_BASELINE_SAMPLES`] — adaptive thresholds disabled.
    pub fn pre_warmup(&self) -> bool {
        self.samples.len() < MIN_BASELINE_SAMPLES
    }

    /// Sample mean. `0.0` when the window is empty.
    pub fn mean(&self) -> f64 {
        if self.samples.is_empty() {
            return 0.0;
        }
        let sum: f64 = self.samples.iter().map(|&s| s as f64).sum();
        sum / self.samples.len() as f64
    }

    /// Sample standard deviation (Bessel-corrected). `0.0` for n ≤ 1.
    pub fn stddev(&self) -> f64 {
        let n = self.samples.len();
        if n <= 1 {
            return 0.0;
        }
        let mean = self.mean();
        let var: f64 = self
            .samples
            .iter()
            .map(|&s| {
                let d = s as f64 - mean;
                d * d
            })
            .sum::<f64>()
            / (n - 1) as f64;
        var.sqrt()
    }
}

/// Layered alert policy with hysteresis + min-dwell + 4-tier severity
/// + rolling-baseline adaptation (NI1 SC9).
///
/// Field privacy (NI6): all configuration knobs are crate-private and
/// reachable only through the validating builders. Builders return
/// `Result<Self, PolicyError>` so callers cannot construct an
/// inconsistent policy. The legacy non-fallible builders are
/// preserved as wrappers that `panic!` on invalid input — these are
/// used in tests and in code paths where the inputs are static
/// constants known to be valid; mark them with `expect` rather than
/// silently swallowing errors.
#[derive(Debug, Clone)]
pub struct LayeredAlertPolicy {
    /// Hard token budget ceiling. Crossing this fires a `Warning`
    /// and triggers the kill-switch (the engine refuses subsequent
    /// mutating operations until the user master-acks or raises
    /// the budget).
    pub(crate) budget_ceiling: u64,
    /// How many consecutive ticks the underlying condition must
    /// hold before the alert fires.
    pub(crate) min_dwell_ticks: u32,
    /// Threshold for the `Advisory` tier (default 20K).
    pub(crate) advisory_threshold: u64,
    /// Threshold for the `Caution` tier (default 50K).
    pub(crate) caution_threshold: u64,
    /// Rolling-baseline window. Wrapped in `Arc<Mutex<...>>` so the
    /// policy can record samples through `&self` (the trait method
    /// signature is immutable). Spec § 4.3 (NI1 SC9).
    pub(crate) baseline: Arc<Mutex<BaselineWindow>>,
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
            baseline: Arc::new(Mutex::new(BaselineWindow::new())),
        }
    }

    /// Validate the current configuration and return self on success.
    /// Tier ordering must satisfy `advisory < caution < budget` and
    /// `min_dwell >= 1`. Spec § 5.3 (NI6 field-privacy + ordering).
    pub fn validate(self) -> Result<Self, PolicyError> {
        if self.min_dwell_ticks == 0 {
            return Err(PolicyError::ZeroDwell);
        }
        if self.advisory_threshold >= self.caution_threshold
            || self.caution_threshold >= self.budget_ceiling
        {
            return Err(PolicyError::MisorderedTiers);
        }
        Ok(self)
    }

    /// Builder: override min-dwell timer. Validates ordering before
    /// returning the new policy. Use [`LayeredAlertPolicy::with_min_dwell`]
    /// for the legacy infallible variant (panics on invalid input).
    pub fn try_with_min_dwell(mut self, ticks: u32) -> Result<Self, PolicyError> {
        self.min_dwell_ticks = ticks;
        self.validate()
    }

    /// Builder: override the advisory threshold (validating). See
    /// [`Self::try_with_min_dwell`].
    pub fn try_with_advisory_threshold(mut self, t: u64) -> Result<Self, PolicyError> {
        self.advisory_threshold = t;
        self.validate()
    }

    /// Builder: override the caution threshold (validating). See
    /// [`Self::try_with_min_dwell`].
    pub fn try_with_caution_threshold(mut self, t: u64) -> Result<Self, PolicyError> {
        self.caution_threshold = t;
        self.validate()
    }

    /// Override min-dwell timer. Panics on `ticks == 0`.
    pub fn with_min_dwell(self, ticks: u32) -> Self {
        self.try_with_min_dwell(ticks)
            .expect("invalid min_dwell_ticks")
    }

    /// Override the advisory threshold. Panics on tier-ordering violation.
    pub fn with_advisory_threshold(self, t: u64) -> Self {
        self.try_with_advisory_threshold(t)
            .expect("advisory threshold violates tier ordering")
    }

    /// Override the caution threshold. Panics on tier-ordering violation.
    pub fn with_caution_threshold(self, t: u64) -> Self {
        self.try_with_caution_threshold(t)
            .expect("caution threshold violates tier ordering")
    }

    /// Read-only accessor for the hard budget ceiling.
    pub fn budget_ceiling(&self) -> u64 {
        self.budget_ceiling
    }

    /// Read-only accessor for the configured min-dwell.
    pub fn min_dwell_ticks(&self) -> u32 {
        self.min_dwell_ticks
    }

    /// Read-only accessor for the static advisory threshold.
    pub fn advisory_threshold(&self) -> u64 {
        self.advisory_threshold
    }

    /// Read-only accessor for the static caution threshold.
    pub fn caution_threshold(&self) -> u64 {
        self.caution_threshold
    }

    /// Returns `true` if the current token total has breached the
    /// hard kill-switch threshold (≥100% of budget).
    pub fn kill_switch_engaged(&self, token_total: u64) -> bool {
        token_total >= self.budget_ceiling
    }

    /// Resolve effective tier thresholds, blending static configuration
    /// with the rolling-baseline window when post-warmup. Returns
    /// `(advisory, caution)`.
    ///
    /// Pre-warmup (n < [`MIN_BASELINE_SAMPLES`]) → static thresholds.
    /// Post-warmup → `max(static, mean + k·σ)` per tier so the
    /// adaptive scheme can only **raise** the floor when activity is
    /// noisier than the static threshold expects (avoids raining
    /// alerts during a high-baseline workload). When the workload is
    /// quieter, the static threshold still gates so a rare spike still
    /// surfaces. Spec § 4.3 (NI1 SC9). The simplest viable adaptive
    /// scheme; documented inline so future iterations can swap in
    /// e.g. EWMA without touching the call sites.
    pub(crate) fn effective_thresholds(&self) -> (u64, u64) {
        let baseline = self.baseline.lock();
        if baseline.pre_warmup() {
            return (self.advisory_threshold, self.caution_threshold);
        }
        let mean = baseline.mean();
        let stddev = baseline.stddev();
        let advisory_dyn = (mean + ADVISORY_SIGMA_K * stddev).max(0.0) as u64;
        let caution_dyn = (mean + CAUTION_SIGMA_K * stddev).max(0.0) as u64;
        (
            advisory_dyn.max(self.advisory_threshold),
            caution_dyn.max(self.caution_threshold),
        )
    }

    /// Classify a token total into `(severity, band)` if alertable.
    /// Returns `None` for totals below the lowest tier threshold.
    fn classify_token_total(&self, total: u64) -> Option<(Severity, AlertBand)> {
        // Helper: build a one-sided high-band with the standard hysteresis-clear ratio.
        // Static thresholds are validated by construction so `expect` is safe.
        fn band_for(high: f64) -> AlertBand {
            AlertBand::new(0.0, 0.0, high, high * HYSTERESIS_CLEAR_RATIO)
                .expect("static thresholds are valid (non-NaN, ordered)")
        }

        let budget = self.budget_ceiling as f64;
        let warning_threshold = budget * WARNING_BUDGET_FRACTION;
        let (advisory_eff, caution_eff) = self.effective_thresholds();

        // Hard kill-switch: at or above ceiling.
        if total >= self.budget_ceiling {
            return Some((Severity::Warning, band_for(budget)));
        }

        // Soft warning at 80% of budget.
        if (total as f64) >= warning_threshold {
            return Some((Severity::Warning, band_for(warning_threshold)));
        }

        // Caution at MCP-overhead range (or rolling-baseline floor).
        if total >= caution_eff {
            return Some((Severity::Caution, band_for(caution_eff as f64)));
        }

        // Advisory at quadratic inflection (or rolling-baseline floor).
        if total >= advisory_eff {
            return Some((Severity::Advisory, band_for(advisory_eff as f64)));
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
        history: &mut AlertHistory,
    ) -> Vec<Alert> {
        let mut alerts = Vec::new();
        let total = curr.token_ledger.total;
        // NI1 SC9: feed the rolling baseline with each tick's sample.
        // Sampling happens before classification so the in-progress
        // tick contributes to subsequent ticks but not its own
        // threshold evaluation (avoids self-attribution in spike
        // detection). Lock acquisition is contention-free on the
        // engine's per-tick path.
        self.baseline.lock().push(total);
        let classification = self.classify_token_total(total);
        let active_fingerprint = classification
            .as_ref()
            .map(|(sev, _)| Self::fingerprint_for(*sev).to_string());

        // Hysteresis enforcement (B6 re-arm gate, spec § 3.4):
        // any tracked fingerprint whose condition is no longer
        // classified as active is checked against its remembered
        // `high_clear`. The signal must drop to or below `high_clear`
        // for the alarm to be considered *truly* cleared (dwell reset
        // and `cleared` flag set). A brief dip into the hysteresis
        // zone — signal in `(high_clear, high)` — is *not* a clear:
        // the alarm event persists, dwell is preserved, and re-arm
        // is unnecessary because the alarm never actually disengaged.
        //
        // Without this, an oscillating signal that bounces between
        // `high` and just-above-`high_clear` would alternately reset
        // and re-accumulate dwell, defeating both min-dwell and
        // hysteresis. With it, a true clear requires re-crossing the
        // `high_clear` boundary (matches FAA AC 25.1322-1 § 5.3
        // dwell semantics + ISA-18.2 alarm management).
        let signal = total as f64;
        for (fp, entry) in history.fingerprints.iter_mut() {
            if active_fingerprint.as_deref() != Some(fp.as_str()) {
                let truly_cleared = entry.last_high_clear.map(|hc| signal <= hc).unwrap_or(true);
                if truly_cleared {
                    entry.dwell_ticks = 0;
                    entry.cleared = true;
                }
                // else: keep dwell + cleared=false; signal is
                // still inside the hysteresis zone, alarm remains
                // logically active.
            }
        }

        if let Some((severity, band)) = classification {
            let fingerprint = Self::fingerprint_for(severity);

            // Increment dwell on every tick where the condition holds,
            // regardless of whether the alert ultimately fires. This
            // is what lets min_dwell > 1 actually fire.
            let entry = history
                .fingerprints
                .entry(fingerprint.to_string())
                .or_insert(AlertState {
                    fired_at_ms: curr.timestamp_ms,
                    dwell_ticks: 0,
                    cleared: false,
                    last_high_clear: None,
                });
            entry.dwell_ticks = entry.dwell_ticks.saturating_add(1);
            entry.cleared = false;
            // Remember the band's `high_clear` so subsequent ticks
            // can decide whether a non-classified signal counts as
            // a true clear (B6 re-arm gate, spec § 3.4).
            entry.last_high_clear = Some(band.high_clear());
            let dwell_ticks = entry.dwell_ticks;

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
                last_high_clear: None,
            },
        );
        h
    }

    #[test]
    fn below_advisory_emits_no_alert() {
        let policy = LayeredAlertPolicy::new(100_000);
        let prev = snapshot_with_tokens(0);
        let curr = snapshot_with_tokens(10_000);
        let alerts = policy.evaluate(&prev, &curr, &mut AlertHistory::new());
        assert!(alerts.is_empty());
    }

    #[test]
    fn advisory_threshold_with_insufficient_dwell_emits_no_alert() {
        // First-tick observation: dwell becomes 1, less than default min 2.
        let policy = LayeredAlertPolicy::new(100_000);
        let prev = snapshot_with_tokens(10_000);
        let curr = snapshot_with_tokens(25_000);
        let alerts = policy.evaluate(&prev, &curr, &mut AlertHistory::new());
        assert!(alerts.is_empty());
    }

    #[test]
    fn advisory_threshold_with_sufficient_dwell_fires() {
        let policy = LayeredAlertPolicy::new(100_000);
        let prev = snapshot_with_tokens(20_000);
        let curr = snapshot_with_tokens(25_000);
        let mut history = history_with_dwell("token-budget-advisory", 1);
        let alerts = policy.evaluate(&prev, &curr, &mut history);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Advisory));
    }

    #[test]
    fn caution_threshold_classifies_as_caution() {
        let policy = LayeredAlertPolicy::new(200_000);
        let curr = snapshot_with_tokens(60_000);
        let mut history = history_with_dwell("token-budget-caution", 1);
        let alerts = policy.evaluate(&snapshot_with_tokens(50_000), &curr, &mut history);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Caution));
    }

    #[test]
    fn warning_at_eighty_percent_of_budget() {
        // budget = 100K → warning threshold = 80K
        let policy = LayeredAlertPolicy::new(100_000);
        let curr = snapshot_with_tokens(85_000);
        let mut history = history_with_dwell("token-budget-warning", 1);
        let alerts = policy.evaluate(&snapshot_with_tokens(70_000), &curr, &mut history);
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Warning));
    }

    #[test]
    fn warning_at_one_hundred_percent_engages_kill_switch() {
        let policy = LayeredAlertPolicy::new(100_000);
        let curr = snapshot_with_tokens(100_000);
        let mut history = history_with_dwell("token-budget-warning", 1);
        let alerts = policy.evaluate(&snapshot_with_tokens(95_000), &curr, &mut history);
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
        let alerts = policy.evaluate(&prev, &curr, &mut AlertHistory::new());
        assert_eq!(alerts.len(), 1);
    }

    #[test]
    fn alert_carries_hysteresis_band() {
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(1);
        let curr = snapshot_with_tokens(25_000);
        let alerts = policy.evaluate(&snapshot_with_tokens(0), &curr, &mut AlertHistory::new());
        let band = alerts[0].band.expect("band present");
        // Advisory threshold 20K, clear ratio 0.95 → high_clear = 19K
        assert_eq!(band.high(), 20_000.0);
        assert!((band.high_clear() - 19_000.0).abs() < 1e-9);
    }

    #[test]
    fn alert_dwell_increments_with_history() {
        let policy = LayeredAlertPolicy::new(100_000);
        let curr = snapshot_with_tokens(25_000);
        let mut history = history_with_dwell("token-budget-advisory", 5);
        let alerts = policy.evaluate(&snapshot_with_tokens(20_000), &curr, &mut history);
        assert_eq!(alerts[0].dwell_ticks, 6);
    }

    #[test]
    fn budget_ordering_warning_outranks_caution() {
        // At 80K with budget 100K, the warning band must fire (not caution),
        // because 80K is above the warning threshold AND above the caution
        // threshold. Severity classification picks the highest tier.
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(1);
        let curr = snapshot_with_tokens(80_000);
        let alerts = policy.evaluate(&snapshot_with_tokens(0), &curr, &mut AlertHistory::new());
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Warning));
    }

    // ---------- B6: high_clear re-arm gate (spec § 3.4) ----------

    #[test]
    fn b6_signal_dipping_into_hysteresis_zone_does_not_clear() {
        // Spec § 3.4 (B6): a brief dip into (high_clear, high) is *not*
        // a true clear. The alarm event persists, dwell is preserved,
        // and the alarm continues to fire on the next high crossing.
        // Default advisory: high=20_000, high_clear=19_000.
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(2);
        let mut history = AlertHistory::new();

        // Tick 1: 22K → classify as advisory, dwell=1 (no fire yet).
        let _ = policy.evaluate(
            &snapshot_with_tokens(0),
            &snapshot_with_tokens(22_000),
            &mut history,
        );
        // Tick 2: 22K → dwell=2 → fires.
        let alerts = policy.evaluate(
            &snapshot_with_tokens(22_000),
            &snapshot_with_tokens(22_000),
            &mut history,
        );
        assert_eq!(alerts.len(), 1);

        // Tick 3: 19_500 (just-above high_clear of 19_000): not classified
        // (below firing threshold) but signal stayed above high_clear.
        // Per B6 the dwell must be preserved, not reset.
        let _ = policy.evaluate(
            &snapshot_with_tokens(22_000),
            &snapshot_with_tokens(19_500),
            &mut history,
        );
        let entry = history
            .fingerprints
            .get("token-budget-advisory")
            .expect("advisory tracked");
        assert!(
            entry.dwell_ticks > 0,
            "dwell must persist on dip into hysteresis zone, got {}",
            entry.dwell_ticks
        );

        // Tick 4: 22K again → dwell increments → fires immediately.
        let alerts = policy.evaluate(
            &snapshot_with_tokens(19_500),
            &snapshot_with_tokens(22_000),
            &mut history,
        );
        assert_eq!(
            alerts.len(),
            1,
            "alarm should re-fire after dip-into-hysteresis (no true clear)"
        );
    }

    #[test]
    fn b6_signal_below_high_clear_truly_clears() {
        // Spec § 3.4 (B6): a drop to or below high_clear is a true
        // clear — dwell resets and the alarm must re-accumulate
        // min_dwell ticks before firing again.
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(2);
        let mut history = AlertHistory::new();

        // Build dwell to firing.
        let _ = policy.evaluate(
            &snapshot_with_tokens(0),
            &snapshot_with_tokens(22_000),
            &mut history,
        );
        let alerts = policy.evaluate(
            &snapshot_with_tokens(22_000),
            &snapshot_with_tokens(22_000),
            &mut history,
        );
        assert_eq!(alerts.len(), 1);

        // Tick: 18K (below high_clear of 19K) → true clear.
        let _ = policy.evaluate(
            &snapshot_with_tokens(22_000),
            &snapshot_with_tokens(18_000),
            &mut history,
        );
        let entry = history
            .fingerprints
            .get("token-budget-advisory")
            .expect("advisory tracked");
        assert_eq!(
            entry.dwell_ticks, 0,
            "dwell must reset on true clear (signal <= high_clear)"
        );
        assert!(entry.cleared, "cleared flag must be set on true clear");

        // Re-arm requires re-accumulating min_dwell. 22K once → dwell=1, no fire.
        let alerts = policy.evaluate(
            &snapshot_with_tokens(18_000),
            &snapshot_with_tokens(22_000),
            &mut history,
        );
        assert!(
            alerts.is_empty(),
            "single tick at 22K post-clear should not fire (dwell=1 < min_dwell=2)"
        );
    }

    #[test]
    fn b6_alert_state_tracks_high_clear_after_first_fire() {
        // After firing, the state must remember the band's high_clear
        // so subsequent ticks can apply the B6 gate correctly.
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(1);
        let mut history = AlertHistory::new();
        let _ = policy.evaluate(
            &snapshot_with_tokens(0),
            &snapshot_with_tokens(22_000),
            &mut history,
        );
        let entry = history
            .fingerprints
            .get("token-budget-advisory")
            .expect("advisory tracked");
        let high_clear = entry.last_high_clear.expect("high_clear remembered");
        assert!(
            (high_clear - 19_000.0).abs() < 1e-9,
            "expected high_clear=19000 (=20000*0.95), got {high_clear}"
        );
    }

    #[test]
    fn configurable_thresholds_override_defaults() {
        let policy = LayeredAlertPolicy::new(1_000_000)
            .with_advisory_threshold(5_000)
            .with_caution_threshold(15_000)
            .with_min_dwell(1);
        let curr = snapshot_with_tokens(7_000);
        let alerts = policy.evaluate(&snapshot_with_tokens(0), &curr, &mut AlertHistory::new());
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Advisory));
    }

    // ---------- NI1: rolling baseline (SC9) ----------

    #[test]
    fn ni1_baseline_window_pre_warmup_uses_static_thresholds() {
        // Before MIN_BASELINE_SAMPLES samples accumulate, the policy
        // must use its static thresholds — adaptive scheme disabled.
        let policy = LayeredAlertPolicy::new(100_000).with_min_dwell(1);
        // First evaluate: pushes one sample. Static advisory=20K still applies.
        let alerts = policy.evaluate(
            &snapshot_with_tokens(0),
            &snapshot_with_tokens(25_000),
            &mut AlertHistory::new(),
        );
        assert_eq!(alerts.len(), 1);
        assert!(matches!(alerts[0].severity, Severity::Advisory));
    }

    #[test]
    fn ni1_baseline_window_post_warmup_raises_advisory_floor() {
        // After warmup with a noisy high-mean sample stream, the
        // rolling baseline floor exceeds the static advisory
        // threshold (mean + 2σ > 20K). Use mean ~25K with σ ~2K so
        // the post-warmup advisory floor lands ~29K.
        let policy = LayeredAlertPolicy::new(200_000).with_min_dwell(1);
        let mut history = AlertHistory::new();
        for i in 0..MIN_BASELINE_SAMPLES {
            // Alternate 23K / 27K so mean=25K, sample-stddev ~2K.
            let total = if i % 2 == 0 { 23_000 } else { 27_000 };
            let _ = policy.evaluate(
                &snapshot_with_tokens(0),
                &snapshot_with_tokens(total),
                &mut history,
            );
        }
        let (advisory_eff, _) = policy.effective_thresholds();
        assert!(
            advisory_eff > 20_000,
            "post-warmup advisory floor must exceed static 20K when mean ~25K + 2σ, got {}",
            advisory_eff
        );
    }

    #[test]
    fn ni1_baseline_pre_warmup_falls_back_when_n_below_min() {
        let policy = LayeredAlertPolicy::new(100_000);
        // Drive one less than MIN_BASELINE_SAMPLES.
        let mut history = AlertHistory::new();
        for _ in 0..(MIN_BASELINE_SAMPLES - 1) {
            let _ = policy.evaluate(
                &snapshot_with_tokens(0),
                &snapshot_with_tokens(40_000),
                &mut history,
            );
        }
        let (advisory_eff, caution_eff) = policy.effective_thresholds();
        assert_eq!(
            advisory_eff, TOKEN_ADVISORY_THRESHOLD,
            "pre-warmup must use static advisory"
        );
        assert_eq!(
            caution_eff, TOKEN_CAUTION_THRESHOLD,
            "pre-warmup must use static caution"
        );
    }

    #[test]
    fn ni1_baseline_window_evicts_oldest_at_capacity() {
        let mut bw = BaselineWindow::with_capacity(MIN_BASELINE_SAMPLES);
        for i in 0..(MIN_BASELINE_SAMPLES + 50) {
            bw.push(i as u64);
        }
        assert_eq!(bw.len(), MIN_BASELINE_SAMPLES);
    }

    // ---------- NI6: field privacy + builder validation ----------

    #[test]
    fn ni6_validate_rejects_misordered_tiers() {
        let mut p = LayeredAlertPolicy::new(100_000);
        p.advisory_threshold = 90_000;
        p.caution_threshold = 50_000;
        let err = p.validate().expect_err("misordered must fail");
        assert_eq!(err, PolicyError::MisorderedTiers);
    }

    #[test]
    fn ni6_validate_rejects_zero_dwell() {
        let mut p = LayeredAlertPolicy::new(100_000);
        p.min_dwell_ticks = 0;
        let err = p.validate().expect_err("zero dwell must fail");
        assert_eq!(err, PolicyError::ZeroDwell);
    }

    #[test]
    fn ni6_try_with_caution_below_advisory_returns_err() {
        let p = LayeredAlertPolicy::new(100_000);
        let err = p
            .try_with_caution_threshold(10_000) // below default advisory 20K
            .expect_err("caution < advisory must fail");
        assert_eq!(err, PolicyError::MisorderedTiers);
    }

    #[test]
    fn ni6_try_with_advisory_above_caution_returns_err() {
        let p = LayeredAlertPolicy::new(100_000);
        let err = p
            .try_with_advisory_threshold(60_000) // above default caution 50K
            .expect_err("advisory > caution must fail");
        assert_eq!(err, PolicyError::MisorderedTiers);
    }

    #[test]
    fn ni6_legacy_with_min_dwell_panics_on_zero() {
        let p = LayeredAlertPolicy::new(100_000);
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = p.with_min_dwell(0);
        }));
        assert!(r.is_err(), "infallible builder must panic on invalid input");
    }

    #[test]
    fn ni6_accessors_round_trip_construction() {
        let p = LayeredAlertPolicy::new(100_000);
        assert_eq!(p.budget_ceiling(), 100_000);
        assert_eq!(p.advisory_threshold(), TOKEN_ADVISORY_THRESHOLD);
        assert_eq!(p.caution_threshold(), TOKEN_CAUTION_THRESHOLD);
        assert_eq!(p.min_dwell_ticks(), DEFAULT_MIN_DWELL_TICKS);
    }
}
