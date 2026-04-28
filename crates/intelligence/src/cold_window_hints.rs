//! Hint scoring for the cold-window engine.
//!
//! Implements the `HintScorer` trait declared in
//! `skrills_analyze::cold_window::traits`. The default scoring
//! formula (per `docs/cold-window-spec.md` § 6.3) is:
//!
//! ```text
//! score = (frequency * IMPACT_WEIGHT + impact * ACTIONABILITY_WEIGHT)
//!         / (ease_score + 1.0)
//!         * recency_factor
//!         + user_pin_boost
//!
//! recency_factor = exp(-age_days / HALF_LIFE_DAYS)
//! ```
//!
//! Defaults: `IMPACT_WEIGHT = 2.0`, `ACTIONABILITY_WEIGHT = 1.5`,
//! `HALF_LIFE_DAYS = 14`. Pin status is a primary sort key (pinned
//! hints always rank above non-pinned regardless of score), not a
//! score boost.
//!
//! Override use cases (per spec): severity-first ordering for
//! incident response, ease-first ordering for "low-hanging fruit"
//! mode. Customize by constructing [`MultiSignalScorer`] with
//! different weights or implementing your own `HintScorer`.
//!
//! This module does not depend on `skrills_analyze`. The
//! `HintScorer` trait lives in `skrills_analyze::cold_window::traits`;
//! `skrills_analyze::cold_window::engine::DefaultHintScorer` is the
//! adapter that wraps [`MultiSignalScorer`] for the engine — see its
//! definition for the boundary contract.

use skrills_snapshot::{Hint, ScoredHint};

/// Weight applied to the `frequency` signal in the default formula.
pub const IMPACT_WEIGHT: f64 = 2.0;

/// Weight applied to the `impact` signal in the default formula.
pub const ACTIONABILITY_WEIGHT: f64 = 1.5;

/// Half-life (days) for the recency decay term.
pub const HALF_LIFE_DAYS: f64 = 14.0;

/// Default cold-window hint scorer.
///
/// Construct via [`MultiSignalScorer::new`] for spec defaults, or
/// the builder methods to override individual weights.
#[derive(Debug, Clone, Copy)]
pub struct MultiSignalScorer {
    /// Weight applied to the frequency signal.
    pub impact_weight: f64,
    /// Weight applied to the impact signal.
    pub actionability_weight: f64,
    /// Half-life in days for recency decay.
    pub half_life_days: f64,
}

impl MultiSignalScorer {
    /// Construct with the cold-window-spec defaults.
    pub fn new() -> Self {
        Self {
            impact_weight: IMPACT_WEIGHT,
            actionability_weight: ACTIONABILITY_WEIGHT,
            half_life_days: HALF_LIFE_DAYS,
        }
    }

    /// Override the frequency weight.
    pub fn with_impact_weight(mut self, w: f64) -> Self {
        self.impact_weight = w;
        self
    }

    /// Override the impact (actionability) weight.
    pub fn with_actionability_weight(mut self, w: f64) -> Self {
        self.actionability_weight = w;
        self
    }

    /// Override the recency half-life in days.
    pub fn with_half_life_days(mut self, d: f64) -> Self {
        self.half_life_days = d;
        self
    }

    /// Compute the unpinned score for a single hint.
    ///
    /// Pin status is intentionally not part of the numeric score —
    /// pinned hints sort ahead of unpinned in [`Self::rank_with_pins`]
    /// regardless of how high the unpinned hint scores.
    pub fn score_one(&self, hint: &Hint) -> f64 {
        let numerator =
            (hint.frequency as f64) * self.impact_weight + hint.impact * self.actionability_weight;
        let denominator = hint.ease_score + 1.0;
        let recency = (-hint.age_days / self.half_life_days.max(f64::MIN_POSITIVE)).exp();
        (numerator / denominator) * recency
    }

    /// Rank `hints` by computed score, highest first. Pinned hints
    /// stick to the top regardless of their base score.
    ///
    /// Pin status is not present in the input `Hint` struct (it lives
    /// on the user's local pin file). Callers that have pin state
    /// should use [`Self::rank_with_pins`]; the bare [`Self::rank`]
    /// treats all hints as unpinned.
    pub fn rank(&self, hints: Vec<Hint>) -> Vec<ScoredHint> {
        self.rank_with_pins(hints, |_| false)
    }

    /// Rank with explicit per-hint pin lookup.
    ///
    /// Sort order: pinned hints first (regardless of score), then by
    /// score descending within each group. Pin status is a primary
    /// sort key, not a score boost — this matches the spec
    /// requirement that pinned hints "stick to the top regardless of
    /// score".
    pub fn rank_with_pins<F>(&self, hints: Vec<Hint>, is_pinned: F) -> Vec<ScoredHint>
    where
        F: Fn(&Hint) -> bool,
    {
        let mut scored: Vec<ScoredHint> = hints
            .into_iter()
            .map(|hint| {
                let pinned = is_pinned(&hint);
                let score = self.score_one(&hint);
                ScoredHint {
                    hint,
                    score,
                    pinned,
                }
            })
            .collect();
        scored.sort_by(|a, b| {
            // Primary: pinned (true) before unpinned (false).
            // Secondary: score descending.
            b.pinned.cmp(&a.pinned).then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        scored
    }
}

impl Default for MultiSignalScorer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::HintCategory;

    fn hint(uri: &str, frequency: u32, impact: f64, ease: f64, age: f64) -> Hint {
        Hint {
            uri: uri.to_string(),
            category: HintCategory::Token,
            message: "test".into(),
            frequency,
            impact,
            ease_score: ease,
            age_days: age,
        }
    }

    #[test]
    fn defaults_match_spec() {
        let s = MultiSignalScorer::new();
        assert_eq!(s.impact_weight, 2.0);
        assert_eq!(s.actionability_weight, 1.5);
        assert_eq!(s.half_life_days, 14.0);
    }

    #[test]
    fn higher_frequency_outranks_lower_when_others_equal() {
        let s = MultiSignalScorer::new();
        let h_low = hint("low", 1, 5.0, 5.0, 0.0);
        let h_hi = hint("hi", 10, 5.0, 5.0, 0.0);
        assert!(s.score_one(&h_hi) > s.score_one(&h_low));
    }

    #[test]
    fn higher_impact_outranks_lower_when_others_equal() {
        let s = MultiSignalScorer::new();
        let h_low = hint("low", 5, 1.0, 5.0, 0.0);
        let h_hi = hint("hi", 5, 9.0, 5.0, 0.0);
        assert!(s.score_one(&h_hi) > s.score_one(&h_low));
    }

    #[test]
    fn higher_ease_lowers_priority() {
        // Ease is in the denominator — high ease means "easy fix,
        // don't surface loudly" so the score goes down. This matches
        // the spec formula: frequency * impact / ease emphasizes
        // hints that are simultaneously high-frequency, high-impact,
        // and hard to fix (i.e., the ones that matter most).
        let s = MultiSignalScorer::new();
        let h_hard = hint("hard", 5, 5.0, 1.0, 0.0);
        let h_easy = hint("easy", 5, 5.0, 9.0, 0.0);
        assert!(s.score_one(&h_easy) < s.score_one(&h_hard));
    }

    #[test]
    fn newer_hint_outranks_older_when_others_equal() {
        let s = MultiSignalScorer::new();
        let h_old = hint("old", 5, 5.0, 5.0, 30.0);
        let h_new = hint("new", 5, 5.0, 5.0, 0.0);
        assert!(s.score_one(&h_new) > s.score_one(&h_old));
    }

    #[test]
    fn pinned_hint_outranks_any_unpinned_in_ranking() {
        // Pin status is a primary sort key, not a score boost.
        // Even a weak pinned hint must sort ahead of a strong unpinned hint.
        let s = MultiSignalScorer::new();
        let weak = hint("weak", 1, 0.1, 9.0, 60.0);
        let strong = hint("strong", 100, 10.0, 1.0, 0.0);
        let ranked = s.rank_with_pins(vec![strong, weak], |h| h.uri == "weak");
        assert_eq!(ranked[0].hint.uri, "weak");
        assert!(ranked[0].pinned);
        assert!(!ranked[1].pinned);
    }

    #[test]
    fn rank_sorts_descending_by_score() {
        let s = MultiSignalScorer::new();
        let hints = vec![
            hint("a", 1, 1.0, 5.0, 0.0),
            hint("b", 10, 9.0, 1.0, 0.0),
            hint("c", 5, 5.0, 5.0, 0.0),
        ];
        let ranked = s.rank(hints);
        assert!(ranked[0].score >= ranked[1].score);
        assert!(ranked[1].score >= ranked[2].score);
        assert_eq!(ranked[0].hint.uri, "b");
    }

    #[test]
    fn rank_with_pins_uses_predicate() {
        let s = MultiSignalScorer::new();
        let hints = vec![
            hint("plain", 100, 10.0, 1.0, 0.0),
            hint("pinned", 1, 0.1, 9.0, 60.0),
        ];
        let ranked = s.rank_with_pins(hints, |h| h.uri == "pinned");
        assert_eq!(ranked[0].hint.uri, "pinned");
        assert!(ranked[0].pinned);
        assert!(!ranked[1].pinned);
    }

    #[test]
    fn empty_hint_list_yields_empty_ranking() {
        let s = MultiSignalScorer::new();
        let ranked = s.rank(vec![]);
        assert!(ranked.is_empty());
    }

    #[test]
    fn custom_weights_change_ranking_predictably() {
        // Make actionability dominant so impact swings the rank.
        let s = MultiSignalScorer::new()
            .with_impact_weight(0.1)
            .with_actionability_weight(10.0);
        let h_freq = hint("freq", 100, 1.0, 5.0, 0.0);
        let h_imp = hint("imp", 1, 9.0, 5.0, 0.0);
        let ranked = s.rank(vec![h_freq, h_imp]);
        assert_eq!(ranked[0].hint.uri, "imp");
    }

    #[test]
    fn recency_decay_is_exponential() {
        let s = MultiSignalScorer::new();
        let h_now = hint("now", 5, 5.0, 5.0, 0.0);
        let h_one_halflife = hint("h1", 5, 5.0, 5.0, 14.0);
        let s_now = s.score_one(&h_now);
        let s_decayed = s.score_one(&h_one_halflife);
        // After one half-life, score is approximately s_now / e (≈0.368).
        let ratio = s_decayed / s_now;
        assert!((ratio - (-1.0_f64).exp()).abs() < 1e-9);
    }

    #[test]
    fn ranking_ties_broken_stably_by_score_within_pin_group() {
        let s = MultiSignalScorer::new();
        let pinned_strong = hint("ps", 50, 8.0, 2.0, 0.0);
        let pinned_weak = hint("pw", 5, 1.0, 9.0, 0.0);
        let unpinned_strong = hint("us", 100, 10.0, 1.0, 0.0);
        let unpinned_weak = hint("uw", 1, 0.5, 9.0, 60.0);
        let ranked = s.rank_with_pins(
            vec![unpinned_weak, pinned_weak, unpinned_strong, pinned_strong],
            |h| h.uri.starts_with('p'),
        );
        // Pinned group first; within it, strong before weak.
        assert_eq!(ranked[0].hint.uri, "ps");
        assert_eq!(ranked[1].hint.uri, "pw");
        // Unpinned group second; within it, strong before weak.
        assert_eq!(ranked[2].hint.uri, "us");
        assert_eq!(ranked[3].hint.uri, "uw");
    }
}
