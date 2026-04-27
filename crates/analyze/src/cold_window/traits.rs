//! Strategy traits for the cold-window engine.
//!
//! Each trait declares a contract; default implementations land in
//! later cold-window plan tasks:
//!
//! | Trait | Default impl | Plan task |
//! |---|---|---|
//! | [`AlertPolicy`] | `LayeredAlertPolicy` | TASK-013 |
//! | [`HintScorer`] | `MultiSignalScorer`  | TASK-010 |
//! | [`ResearchBudget`] | `BucketedBudget` | TASK-011 |
//! | [`SnapshotDiff`] | `FieldwiseDiff`    | TASK-014 |
//!
//! All traits are kept object-safe so the engine can store them as
//! `Box<dyn Trait>` and accept user overrides at runtime. The
//! `traits_are_object_safe` test below exists exactly to catch any
//! accidental loss of object-safety during evolution.

use std::collections::HashMap;
use std::time::Instant;

use skrills_snapshot::{Alert, Hint, ScoredHint, WindowSnapshot};

/// Per-alert hysteresis state carried forward across ticks.
#[derive(Debug, Clone)]
pub struct AlertState {
    /// First-fire wall-clock (UNIX epoch ms).
    pub fired_at_ms: u64,
    /// How many consecutive ticks the underlying condition has held.
    pub dwell_ticks: u32,
    /// True after the alert has been cleared (re-armed when the
    /// condition re-crosses the matching `*_clear` threshold in
    /// the alert's [`skrills_snapshot::AlertBand`]).
    pub cleared: bool,
}

/// Hysteresis state across all known alert fingerprints.
#[derive(Debug, Default, Clone)]
pub struct AlertHistory {
    /// Per-fingerprint state.
    pub fingerprints: HashMap<String, AlertState>,
}

impl AlertHistory {
    /// Construct an empty history (warmup state).
    pub fn new() -> Self {
        Self::default()
    }
}

/// Decide which alerts fire on each tick.
///
/// Implementations apply hysteresis bands, min-dwell timers, and the
/// 4-tier severity classification per spec § 3.4. The default
/// implementation `LayeredAlertPolicy` lands in TASK-013.
///
/// Mutates `history` so dwell counters increment even on ticks where
/// the condition holds but min-dwell has not yet been satisfied. This
/// is what lets a `min_dwell = 2` policy actually fire on the second
/// tick that observes the condition rather than waiting forever.
pub trait AlertPolicy: Send + Sync {
    /// Evaluate alerts and update history. Returns the alert list to
    /// be included in `curr.alerts`.
    fn evaluate(
        &self,
        prev: &WindowSnapshot,
        curr: &WindowSnapshot,
        history: &mut AlertHistory,
    ) -> Vec<Alert>;
}

/// Rank a list of hints into a `ScoredHint` vector.
///
/// The default implementation `MultiSignalScorer` (TASK-010) extends
/// the existing `skrills_intelligence::recommend::scorer` machinery
/// with a recency-weighted ratio per spec § 6.3.
pub trait HintScorer: Send + Sync {
    /// Compute scores and return hints ranked from highest score to
    /// lowest. Pinned hints sort to the top regardless of score.
    fn rank(&self, hints: Vec<Hint>) -> Vec<ScoredHint>;
}

/// Decide whether the research dispatcher should issue an external
/// fetch for a given topic fingerprint.
///
/// The default implementation `BucketedBudget` (TASK-011) enforces a
/// token-bucket capacity, per-fingerprint TTL, and persistence
/// across daemon restarts (R10 mitigation).
pub trait ResearchBudget: Send + Sync {
    /// Return true when an external fetch is permitted for the given
    /// fingerprint at this moment, false when the budget refuses.
    fn should_query(
        &self,
        snapshot: &WindowSnapshot,
        topic_fingerprint: &str,
        last_query: Option<Instant>,
    ) -> bool;
}

/// One field that changed between two snapshots and is considered
/// alertable by the active [`SnapshotDiff`] policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffField {
    /// Total token count changed beyond the configured tolerance.
    TokenTotal {
        /// Previous total tokens.
        before: u64,
        /// New total tokens.
        after: u64,
    },
    /// A skill appeared in the snapshot.
    SkillAdded(String),
    /// A skill was removed from the snapshot.
    SkillRemoved(String),
    /// A plugin appeared in the snapshot.
    PluginAdded(String),
    /// A plugin was removed from the snapshot.
    PluginRemoved(String),
    /// A skill's validation status flipped.
    ValidationTransition {
        /// Skill URI.
        uri: String,
        /// Previous validity.
        from: bool,
        /// New validity.
        to: bool,
    },
}

/// Decide what fields changed enough between two snapshots to be
/// worth alerting on.
///
/// The default implementation `FieldwiseDiff` (TASK-014) applies
/// declarative per-field rules: ±2% tolerance on token counts,
/// always-alert on skill/plugin add/remove, never-alert on
/// timestamps.
pub trait SnapshotDiff: Send + Sync {
    /// Return the alertable fields between `prev` and `curr`. An
    /// empty vector means nothing changed enough to warrant alert
    /// evaluation this tick.
    fn is_alertable(&self, prev: &WindowSnapshot, curr: &WindowSnapshot) -> Vec<DiffField>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time guard: each strategy trait must remain object-safe
    /// so the engine can hold them as `Box<dyn Trait>`. If any of
    /// these `dyn` references stops compiling, an unintended
    /// non-object-safe change has been introduced (e.g. a generic
    /// method, a `Self` return type, or an associated constant
    /// lacking a default).
    #[test]
    fn traits_are_object_safe() {
        fn assert_object_safe<T: ?Sized>() {}
        assert_object_safe::<dyn AlertPolicy>();
        assert_object_safe::<dyn HintScorer>();
        assert_object_safe::<dyn ResearchBudget>();
        assert_object_safe::<dyn SnapshotDiff>();
    }

    #[test]
    fn alert_history_starts_empty() {
        let h = AlertHistory::new();
        assert!(h.fingerprints.is_empty());
    }

    #[test]
    fn diff_field_partial_eq_is_structural() {
        let a = DiffField::SkillAdded("skill://demo".into());
        let b = DiffField::SkillAdded("skill://demo".into());
        let c = DiffField::SkillAdded("skill://other".into());
        assert_eq!(a, b);
        assert_ne!(a, c);
    }
}
