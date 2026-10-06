//! Default snapshot diff policy for the cold-window engine.
//!
//! [`FieldwiseDiff`] applies
//! per-field rules:
//!
//! - **Token totals**: alert on ±2% change (kills heuristic noise).
//! - **Skill set**: alert on add/remove (any change).
//! - **Plugin set**: alert on add/remove (any change).
//! - **Validation status**: alert on transition (per-check).
//! - **Timestamps**: never alert (always change, never meaningful).
//!
//! Override use cases: stricter mode (every field change alerts) for
//! compliance environments, looser mode (only major adds/removes)
//! for high-churn dev environments.

use std::collections::{HashMap, HashSet};

use skrills_snapshot::{HealthStatus, WindowSnapshot};

use super::traits::{DiffField, SnapshotDiff};

/// Default tolerance for token-count comparisons (±2%).
///
/// Below this ratio a token-count change is treated as estimator
/// noise and not surfaced to the alert layer. Above it, a
/// `DiffField::TokenTotal` is emitted.
pub const TOKEN_TOLERANCE_RATIO: f64 = 0.02;

/// Default fieldwise snapshot diff.
#[derive(Debug, Clone, Copy)]
pub struct FieldwiseDiff {
    /// Tolerance for token-count changes (default: 2%).
    pub token_tolerance: f64,
}

impl FieldwiseDiff {
    /// Construct with the spec default tolerance.
    pub fn new() -> Self {
        Self {
            token_tolerance: TOKEN_TOLERANCE_RATIO,
        }
    }

    /// Override the token tolerance ratio.
    pub fn with_token_tolerance(mut self, tol: f64) -> Self {
        self.token_tolerance = tol;
        self
    }
}

impl Default for FieldwiseDiff {
    fn default() -> Self {
        Self::new()
    }
}

fn collect_sources<'a, I>(entries: I) -> HashSet<&'a str>
where
    I: IntoIterator<Item = &'a skrills_snapshot::TokenEntry>,
{
    entries.into_iter().map(|e| e.source.as_str()).collect()
}

fn validation_map(snap: &WindowSnapshot) -> HashMap<String, bool> {
    snap.plugin_health
        .iter()
        .flat_map(|p| {
            p.checks.iter().map(move |c| {
                (
                    format!("{}#{}", p.plugin_name, c.name),
                    matches!(c.status, HealthStatus::Ok),
                )
            })
        })
        .collect()
}

impl SnapshotDiff for FieldwiseDiff {
    fn is_alertable(&self, prev: &WindowSnapshot, curr: &WindowSnapshot) -> Vec<DiffField> {
        let mut diffs = Vec::new();

        // Token total: alert when delta exceeds the configured tolerance.
        let prev_total = prev.token_ledger.total;
        let curr_total = curr.token_ledger.total;
        if prev_total != curr_total {
            let denom = prev_total.max(1) as f64;
            let abs_delta = (curr_total as i128 - prev_total as i128).unsigned_abs() as f64;
            let delta_ratio = abs_delta / denom;
            if delta_ratio > self.token_tolerance {
                diffs.push(DiffField::TokenTotal {
                    before: prev_total,
                    after: curr_total,
                });
            }
        }

        // Skills: any add/remove alerts (no tolerance).
        let prev_skills = collect_sources(&prev.token_ledger.per_skill);
        let curr_skills = collect_sources(&curr.token_ledger.per_skill);
        for added in curr_skills.difference(&prev_skills) {
            diffs.push(DiffField::SkillAdded((*added).to_string()));
        }
        for removed in prev_skills.difference(&curr_skills) {
            diffs.push(DiffField::SkillRemoved((*removed).to_string()));
        }

        // Plugins: any add/remove alerts (no tolerance).
        let prev_plugins = collect_sources(&prev.token_ledger.per_plugin);
        let curr_plugins = collect_sources(&curr.token_ledger.per_plugin);
        for added in curr_plugins.difference(&prev_plugins) {
            diffs.push(DiffField::PluginAdded((*added).to_string()));
        }
        for removed in prev_plugins.difference(&curr_plugins) {
            diffs.push(DiffField::PluginRemoved((*removed).to_string()));
        }

        // Validation transitions: derived from plugin_health checks.
        // A check missing on one side counts as passing there, so a
        // check that first appears already failing, or a failing check
        // that disappears, still reports a transition.
        let prev_validation = validation_map(prev);
        let curr_validation = validation_map(curr);
        let mut uris: Vec<&String> = prev_validation
            .keys()
            .chain(curr_validation.keys())
            .collect();
        uris.sort();
        uris.dedup();
        for uri in uris {
            let prev_ok = prev_validation.get(uri).copied().unwrap_or(true);
            let curr_ok = curr_validation.get(uri).copied().unwrap_or(true);
            if prev_ok != curr_ok {
                diffs.push(DiffField::ValidationTransition {
                    uri: uri.clone(),
                    from: prev_ok,
                    to: curr_ok,
                });
            }
        }

        diffs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::{
        HealthCheck, HealthStatus, LoadSample, PluginHealth, TokenEntry, TokenLedger,
    };

    fn empty_snapshot() -> WindowSnapshot {
        WindowSnapshot {
            version: 0,
            timestamp_ms: 0,
            token_ledger: TokenLedger::default(),
            alerts: vec![],
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        }
    }

    fn snap_with_total(total: u64) -> WindowSnapshot {
        let mut s = empty_snapshot();
        s.token_ledger.total = total;
        s
    }

    fn snap_with_skills(names: &[&str]) -> WindowSnapshot {
        let mut s = empty_snapshot();
        s.token_ledger.per_skill = names
            .iter()
            .map(|n| TokenEntry {
                source: (*n).to_string(),
                tokens: 100,
            })
            .collect();
        s
    }

    fn snap_with_plugins(names: &[&str]) -> WindowSnapshot {
        let mut s = empty_snapshot();
        s.token_ledger.per_plugin = names
            .iter()
            .map(|n| TokenEntry {
                source: (*n).to_string(),
                tokens: 100,
            })
            .collect();
        s
    }

    #[test]
    fn timestamp_only_change_emits_no_diffs() {
        let mut prev = empty_snapshot();
        prev.timestamp_ms = 1_000;
        let mut curr = empty_snapshot();
        curr.timestamp_ms = 2_000;
        let diffs = FieldwiseDiff::new().is_alertable(&prev, &curr);
        assert!(diffs.is_empty(), "expected no diffs, got {diffs:?}");
    }

    #[test]
    fn token_change_within_tolerance_emits_no_diffs() {
        // 1% change with default 2% tolerance → no alert.
        let prev = snap_with_total(10_000);
        let curr = snap_with_total(10_100);
        let diffs = FieldwiseDiff::new().is_alertable(&prev, &curr);
        assert!(diffs.is_empty());
    }

    #[test]
    fn token_change_beyond_tolerance_emits_token_total() {
        // 5% change with default 2% tolerance → alert.
        let prev = snap_with_total(10_000);
        let curr = snap_with_total(10_500);
        let diffs = FieldwiseDiff::new().is_alertable(&prev, &curr);
        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            diffs[0],
            DiffField::TokenTotal {
                before: 10_000,
                after: 10_500
            }
        ));
    }

    #[test]
    fn skill_addition_emits_skill_added() {
        let prev = snap_with_skills(&["a", "b"]);
        let curr = snap_with_skills(&["a", "b", "c"]);
        let diffs = FieldwiseDiff::new().is_alertable(&prev, &curr);
        assert_eq!(
            diffs
                .iter()
                .filter(|d| matches!(d, DiffField::SkillAdded(_)))
                .count(),
            1
        );
    }

    #[test]
    fn skill_removal_emits_skill_removed() {
        let prev = snap_with_skills(&["a", "b", "c"]);
        let curr = snap_with_skills(&["a", "b"]);
        let diffs = FieldwiseDiff::new().is_alertable(&prev, &curr);
        assert_eq!(
            diffs
                .iter()
                .filter(|d| matches!(d, DiffField::SkillRemoved(_)))
                .count(),
            1
        );
    }

    #[test]
    fn plugin_add_remove_both_emit() {
        let prev = snap_with_plugins(&["alpha", "beta"]);
        let curr = snap_with_plugins(&["beta", "gamma"]);
        let diffs = FieldwiseDiff::new().is_alertable(&prev, &curr);
        let added = diffs
            .iter()
            .filter(|d| matches!(d, DiffField::PluginAdded(_)))
            .count();
        let removed = diffs
            .iter()
            .filter(|d| matches!(d, DiffField::PluginRemoved(_)))
            .count();
        assert_eq!(added, 1);
        assert_eq!(removed, 1);
    }

    fn snap_with_check(status: HealthStatus) -> WindowSnapshot {
        let mut s = empty_snapshot();
        s.plugin_health.push(PluginHealth {
            plugin_name: "p1".into(),
            overall: status,
            checks: vec![HealthCheck {
                name: "schema".into(),
                status,
                message: None,
            }],
        });
        s
    }

    fn transitions(diffs: &[DiffField]) -> Vec<(bool, bool)> {
        diffs
            .iter()
            .filter_map(|d| match d {
                DiffField::ValidationTransition { from, to, .. } => Some((*from, *to)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn check_that_first_appears_failing_emits_a_transition() {
        // IN-66: a check absent before counts as passing, so a new
        // failing check is a true -> false transition.
        let diffs = FieldwiseDiff::new()
            .is_alertable(&empty_snapshot(), &snap_with_check(HealthStatus::Error));
        assert_eq!(transitions(&diffs), [(true, false)]);
    }

    #[test]
    fn failing_check_that_disappears_emits_a_transition() {
        let diffs = FieldwiseDiff::new()
            .is_alertable(&snap_with_check(HealthStatus::Error), &empty_snapshot());
        assert_eq!(transitions(&diffs), [(false, true)]);
    }

    #[test]
    fn passing_check_appearing_or_disappearing_is_not_a_transition() {
        let ok = snap_with_check(HealthStatus::Ok);
        assert!(transitions(&FieldwiseDiff::new().is_alertable(&empty_snapshot(), &ok)).is_empty());
        assert!(transitions(&FieldwiseDiff::new().is_alertable(&ok, &empty_snapshot())).is_empty());
    }

    #[test]
    fn validation_transition_ok_to_error_emits_diff() {
        let mut prev = empty_snapshot();
        prev.plugin_health.push(PluginHealth {
            plugin_name: "p1".into(),
            overall: HealthStatus::Ok,
            checks: vec![HealthCheck {
                name: "schema".into(),
                status: HealthStatus::Ok,
                message: None,
            }],
        });
        let mut curr = empty_snapshot();
        curr.plugin_health.push(PluginHealth {
            plugin_name: "p1".into(),
            overall: HealthStatus::Error,
            checks: vec![HealthCheck {
                name: "schema".into(),
                status: HealthStatus::Error,
                message: Some("parse failed".into()),
            }],
        });
        let diffs = FieldwiseDiff::new().is_alertable(&prev, &curr);
        assert!(diffs.iter().any(|d| matches!(
            d,
            DiffField::ValidationTransition {
                from: true,
                to: false,
                ..
            }
        )));
    }

    #[test]
    fn warmup_state_with_zero_prev_total_handles_gracefully() {
        // prev_total = 0 → denom clamped to 1; first non-zero curr is alertable.
        let prev = snap_with_total(0);
        let curr = snap_with_total(50_000);
        let diffs = FieldwiseDiff::new().is_alertable(&prev, &curr);
        assert!(diffs
            .iter()
            .any(|d| matches!(d, DiffField::TokenTotal { .. })));
    }

    #[test]
    fn custom_tolerance_overrides_default() {
        // 1% change with strict 0.5% tolerance → alert.
        let prev = snap_with_total(10_000);
        let curr = snap_with_total(10_100);
        let diff = FieldwiseDiff::new().with_token_tolerance(0.005);
        let diffs = diff.is_alertable(&prev, &curr);
        assert_eq!(diffs.len(), 1);
    }
}
