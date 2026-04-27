//! Per-tick TUI state for the cold-window panes.
//!
//! `ColdWindowState` ingests `Arc<WindowSnapshot>` from the engine
//! bus and exposes the data shapes each pane needs. It also
//! bookkeeps two pieces of UX state that don't belong on the
//! snapshot:
//!
//! - **Acknowledged warnings**: a set of fingerprints the user has
//!   individually dismissed. WARNING-tier alerts require per-row
//!   ack; the master-ack keystroke does not clear them.
//! - **Acknowledged non-warnings**: a "high water mark" tick version
//!   above which CAUTION/ADVISORY/STATUS alerts are considered
//!   acknowledged.

use std::collections::HashSet;
use std::sync::Arc;

use skrills_snapshot::{Alert, Severity, WindowSnapshot};

/// View-side state used by the cold-window TUI panes.
#[derive(Debug, Default, Clone)]
pub struct ColdWindowState {
    /// The most recent snapshot received on the bus.
    pub current: Option<Arc<WindowSnapshot>>,
    /// Fingerprints of WARNING-tier alerts the user has dismissed.
    /// Stays sticky across ticks (re-firing a warning needs an
    /// alert-history clear from the policy side).
    pub acked_warnings: HashSet<String>,
    /// Snapshot version at which the user last pressed master-ack.
    /// Non-warning alerts at or below this version are filtered out
    /// of the visible list (auto-clear on next tick if the
    /// underlying condition has resolved).
    pub master_ack_version: u64,
    /// Whether to ring the terminal bell when a new WARNING fires.
    /// Maps to the `--no-bell` CLI flag in TASK-021.
    pub bell_enabled: bool,
}

impl ColdWindowState {
    /// Construct a fresh state with the bell enabled by default.
    pub fn new() -> Self {
        Self {
            current: None,
            acked_warnings: HashSet::new(),
            master_ack_version: 0,
            bell_enabled: true,
        }
    }

    /// Apply a new snapshot. Returns `true` when at least one
    /// previously-unseen WARNING-tier alert is present (caller may
    /// ring the bell if [`Self::bell_enabled`] is true).
    pub fn ingest(&mut self, snapshot: Arc<WindowSnapshot>) -> bool {
        let mut new_warning = false;
        for alert in &snapshot.alerts {
            if matches!(alert.severity, Severity::Warning)
                && !self.acked_warnings.contains(&alert.fingerprint)
            {
                new_warning = true;
                break;
            }
        }
        self.current = Some(snapshot);
        new_warning && self.bell_enabled
    }

    /// Clear all visible CAUTION/ADVISORY/STATUS alerts (master-ack).
    /// WARNING-tier alerts remain. Returns the number cleared.
    pub fn master_ack(&mut self) -> usize {
        let snap = match self.current.as_deref() {
            Some(s) => s,
            None => return 0,
        };
        let cleared = snap
            .alerts
            .iter()
            .filter(|a| !matches!(a.severity, Severity::Warning))
            .count();
        self.master_ack_version = snap.version;
        cleared
    }

    /// Acknowledge a single WARNING-tier alert by fingerprint.
    /// Returns true if the alert was newly acknowledged.
    pub fn ack_warning(&mut self, fingerprint: &str) -> bool {
        self.acked_warnings.insert(fingerprint.to_string())
    }

    /// Re-arm a warning fingerprint so a future re-trigger surfaces
    /// again (used when alert history clears via policy).
    pub fn unack_warning(&mut self, fingerprint: &str) -> bool {
        self.acked_warnings.remove(fingerprint)
    }

    /// Visible alerts after applying ack filters and sorting them
    /// by tier (Warning first) then by `fired_at_ms` descending.
    pub fn visible_alerts(&self) -> Vec<&Alert> {
        let snap = match self.current.as_deref() {
            Some(s) => s,
            None => return Vec::new(),
        };
        let mut visible: Vec<&Alert> = snap
            .alerts
            .iter()
            .filter(|a| match a.severity {
                Severity::Warning => !self.acked_warnings.contains(&a.fingerprint),
                _ => snap.version > self.master_ack_version,
            })
            .collect();
        visible.sort_by(|a, b| {
            severity_rank(a.severity)
                .cmp(&severity_rank(b.severity))
                .then(b.fired_at_ms.cmp(&a.fired_at_ms))
        });
        visible
    }

    /// Count visible alerts grouped by tier (for the status bar).
    pub fn alert_counts_by_tier(&self) -> AlertCounts {
        let mut counts = AlertCounts::default();
        for alert in self.visible_alerts() {
            match alert.severity {
                Severity::Warning => counts.warning += 1,
                Severity::Caution => counts.caution += 1,
                Severity::Advisory => counts.advisory += 1,
                Severity::Status => counts.status += 1,
            }
        }
        counts
    }

    /// Total token usage from the most recent snapshot.
    pub fn token_total(&self) -> u64 {
        self.current
            .as_deref()
            .map(|s| s.token_ledger.total)
            .unwrap_or(0)
    }
}

/// Per-tier alert counts for the status bar.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AlertCounts {
    /// Warning-tier count.
    pub warning: usize,
    /// Caution-tier count.
    pub caution: usize,
    /// Advisory-tier count.
    pub advisory: usize,
    /// Status-tier count.
    pub status: usize,
}

impl AlertCounts {
    /// Total count across all tiers.
    pub fn total(&self) -> usize {
        self.warning + self.caution + self.advisory + self.status
    }
}

fn severity_rank(severity: Severity) -> u8 {
    // Lower rank sorts first.
    match severity {
        Severity::Warning => 0,
        Severity::Caution => 1,
        Severity::Advisory => 2,
        Severity::Status => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::{AlertBand, LoadSample, TokenLedger};

    fn alert(fingerprint: &str, severity: Severity, fired_at_ms: u64) -> Alert {
        Alert {
            fingerprint: fingerprint.to_string(),
            severity,
            title: format!("title-{fingerprint}"),
            message: format!("message-{fingerprint}"),
            band: Some(AlertBand {
                low: 0.0,
                low_clear: 0.0,
                high: 1.0,
                high_clear: 0.95,
            }),
            fired_at_ms,
            dwell_ticks: 1,
        }
    }

    fn snapshot(version: u64, alerts: Vec<Alert>) -> Arc<WindowSnapshot> {
        Arc::new(WindowSnapshot {
            version,
            timestamp_ms: 1_700_000_000_000 + version * 2_000,
            token_ledger: TokenLedger {
                total: 25_000,
                ..Default::default()
            },
            alerts,
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        })
    }

    #[test]
    fn fresh_state_has_no_visible_alerts() {
        let s = ColdWindowState::new();
        assert!(s.visible_alerts().is_empty());
    }

    #[test]
    fn ingesting_snapshot_with_no_warnings_does_not_signal_bell() {
        let mut s = ColdWindowState::new();
        let snap = snapshot(1, vec![alert("a1", Severity::Caution, 100)]);
        let bell = s.ingest(snap);
        assert!(!bell);
    }

    #[test]
    fn ingesting_new_warning_signals_bell_when_enabled() {
        let mut s = ColdWindowState::new();
        let snap = snapshot(1, vec![alert("w1", Severity::Warning, 100)]);
        let bell = s.ingest(snap);
        assert!(bell);
    }

    #[test]
    fn ingesting_acked_warning_does_not_signal_bell() {
        let mut s = ColdWindowState::new();
        s.ack_warning("w1");
        let snap = snapshot(1, vec![alert("w1", Severity::Warning, 100)]);
        let bell = s.ingest(snap);
        assert!(!bell);
    }

    #[test]
    fn bell_disabled_suppresses_signal() {
        let mut s = ColdWindowState::new();
        s.bell_enabled = false;
        let snap = snapshot(1, vec![alert("w1", Severity::Warning, 100)]);
        let bell = s.ingest(snap);
        assert!(!bell);
    }

    #[test]
    fn master_ack_clears_non_warnings_only() {
        let mut s = ColdWindowState::new();
        let snap = snapshot(
            1,
            vec![
                alert("w1", Severity::Warning, 100),
                alert("c1", Severity::Caution, 90),
                alert("a1", Severity::Advisory, 80),
                alert("s1", Severity::Status, 70),
            ],
        );
        s.ingest(snap);
        let cleared = s.master_ack();
        assert_eq!(cleared, 3, "should clear caution + advisory + status");
        let visible = s.visible_alerts();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].fingerprint, "w1");
    }

    #[test]
    fn ack_warning_dismisses_individual_warning() {
        let mut s = ColdWindowState::new();
        let snap = snapshot(
            1,
            vec![
                alert("w1", Severity::Warning, 100),
                alert("w2", Severity::Warning, 90),
            ],
        );
        s.ingest(snap);
        assert!(s.ack_warning("w1"));
        let visible = s.visible_alerts();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].fingerprint, "w2");
    }

    #[test]
    fn ack_warning_returns_false_on_repeat() {
        let mut s = ColdWindowState::new();
        assert!(s.ack_warning("w1"));
        assert!(!s.ack_warning("w1"));
    }

    #[test]
    fn unack_warning_re_arms_fingerprint() {
        let mut s = ColdWindowState::new();
        s.ack_warning("w1");
        assert!(s.unack_warning("w1"));
        let snap = snapshot(1, vec![alert("w1", Severity::Warning, 100)]);
        let bell = s.ingest(snap);
        assert!(bell);
    }

    #[test]
    fn visible_alerts_sort_by_tier_then_recency() {
        let mut s = ColdWindowState::new();
        let snap = snapshot(
            1,
            vec![
                alert("a1", Severity::Advisory, 50),
                alert("c1", Severity::Caution, 80),
                alert("w1", Severity::Warning, 100),
                alert("c2", Severity::Caution, 90),
            ],
        );
        s.ingest(snap);
        let visible = s.visible_alerts();
        assert_eq!(visible[0].fingerprint, "w1");
        assert_eq!(visible[1].fingerprint, "c2");
        assert_eq!(visible[2].fingerprint, "c1");
        assert_eq!(visible[3].fingerprint, "a1");
    }

    #[test]
    fn alert_counts_by_tier_aggregate() {
        let mut s = ColdWindowState::new();
        let snap = snapshot(
            1,
            vec![
                alert("w1", Severity::Warning, 100),
                alert("c1", Severity::Caution, 80),
                alert("c2", Severity::Caution, 90),
                alert("a1", Severity::Advisory, 50),
            ],
        );
        s.ingest(snap);
        let counts = s.alert_counts_by_tier();
        assert_eq!(counts.warning, 1);
        assert_eq!(counts.caution, 2);
        assert_eq!(counts.advisory, 1);
        assert_eq!(counts.status, 0);
        assert_eq!(counts.total(), 4);
    }

    #[test]
    fn token_total_reflects_current_snapshot() {
        let mut s = ColdWindowState::new();
        assert_eq!(s.token_total(), 0);
        s.ingest(snapshot(1, vec![]));
        assert_eq!(s.token_total(), 25_000);
    }

    #[test]
    fn master_ack_carries_until_next_tick_with_higher_version() {
        let mut s = ColdWindowState::new();
        let snap1 = snapshot(1, vec![alert("c1", Severity::Caution, 100)]);
        s.ingest(snap1);
        s.master_ack();
        assert!(s.visible_alerts().is_empty());
        // Next tick (version 2) — caution still present, but if it
        // re-fires (a new alert object), version > master_ack_version
        // means it becomes visible again.
        let snap2 = snapshot(2, vec![alert("c1", Severity::Caution, 200)]);
        s.ingest(snap2);
        assert_eq!(s.visible_alerts().len(), 1);
    }
}
