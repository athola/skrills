//! Rolling-baseline queries for adaptive thresholds.
//!
//! Used by the cold-window's `LayeredAlertPolicy` (and any other
//! consumer) to fetch contextual baselines from historical metrics
//! data. Per spec § 8 / Assumption A1: when the rolling window has
//! insufficient samples (warmup state), the query returns `None`
//! so the consumer can fall back to constants instead of producing
//! spurious alerts off a thin sample.
//!
//! No new schema is added — queries run against the existing
//! `skill_invocations`, `validation_runs`, and `rule_triggers`
//! tables.

use std::time::Duration;

use crate::collector::MetricsCollector;
use crate::error::Result;

/// Token usage per skill invocation. Sourced from
/// `skill_invocations.tokens_used`.
pub const METRIC_SKILL_TOKENS: &str = "skill_tokens";

/// Skill invocation duration in milliseconds. Sourced from
/// `skill_invocations.duration_ms`.
pub const METRIC_SKILL_DURATION_MS: &str = "skill_duration_ms";

/// Rule trigger duration in milliseconds. Sourced from
/// `rule_triggers.duration_ms`.
pub const METRIC_RULE_DURATION_MS: &str = "rule_duration_ms";

/// Minimum samples required to produce a non-`None` baseline.
/// Below this, callers should fall back to constants. Set to 5 so a
/// single noisy outlier cannot define the baseline on its own.
pub const MIN_BASELINE_SAMPLES: usize = 5;

/// Trait for fetching rolling baselines.
///
/// Implemented for `MetricsCollector` so callers can take
/// `&dyn BaselineQuery` and substitute in tests.
pub trait BaselineQuery: Send + Sync {
    /// Return the q-quantile of the named metric's values over the
    /// trailing `window`, or `None` when the window has fewer than
    /// [`MIN_BASELINE_SAMPLES`] samples.
    ///
    /// `q` is clamped to `[0.0, 1.0]`. Unknown metric names return
    /// `Ok(None)` (treated as warmup).
    fn quantile_over_window(&self, metric: &str, window: Duration, q: f64) -> Result<Option<f64>>;
}

impl BaselineQuery for MetricsCollector {
    fn quantile_over_window(&self, metric: &str, window: Duration, q: f64) -> Result<Option<f64>> {
        let values = self.collect_metric_values(metric, window)?;
        if values.len() < MIN_BASELINE_SAMPLES {
            return Ok(None);
        }
        let mut values = values;
        Ok(Some(quantile(&mut values, q)))
    }
}

/// Compute the q-quantile of `values` (assumes sortable f64).
///
/// Uses nearest-rank ordering: `values[round((n-1) * q)]`. This is
/// less sophisticated than linear interpolation but plays well with
/// the heuristic-bound nature of token estimation (per spec
/// Assumption A1) — a few extra digits of precision would lie about
/// the underlying signal.
fn quantile(values: &mut [f64], q: f64) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q = q.clamp(0.0, 1.0);
    if values.is_empty() {
        return 0.0;
    }
    let idx = (((values.len() as f64) - 1.0) * q).round() as usize;
    values[idx]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MetricsCollector;

    #[test]
    fn quantile_handles_empty_slice() {
        let mut empty: [f64; 0] = [];
        assert_eq!(quantile(&mut empty, 0.5), 0.0);
    }

    #[test]
    fn quantile_p50_of_odd_length() {
        let mut v = [1.0, 3.0, 5.0, 7.0, 9.0];
        assert_eq!(quantile(&mut v, 0.5), 5.0);
    }

    #[test]
    fn quantile_p95_of_long_series() {
        let mut v: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let p95 = quantile(&mut v, 0.95);
        assert!((p95 - 95.0).abs() <= 1.0);
    }

    #[test]
    fn quantile_clamps_q_to_unit_interval() {
        let mut v = [10.0, 20.0, 30.0];
        assert_eq!(quantile(&mut v, -0.5), 10.0);
        assert_eq!(quantile(&mut v, 1.5), 30.0);
    }

    #[test]
    fn quantile_p0_returns_min_p1_returns_max() {
        let mut v = [3.0, 1.0, 2.0];
        assert_eq!(quantile(&mut v, 0.0), 1.0);
        assert_eq!(quantile(&mut v, 1.0), 3.0);
    }

    #[test]
    fn empty_collector_returns_none_for_warmup() {
        let collector = MetricsCollector::new().unwrap();
        let result = collector
            .quantile_over_window(METRIC_SKILL_TOKENS, Duration::from_secs(3600), 0.5)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn unknown_metric_name_returns_none() {
        let collector = MetricsCollector::new().unwrap();
        let result = collector
            .quantile_over_window("unknown-metric", Duration::from_secs(3600), 0.5)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn collector_with_invocations_produces_baseline() {
        let collector = MetricsCollector::new().unwrap();
        for tokens in [100, 200, 300, 400, 500, 600, 700] {
            collector
                .record_skill_invocation("demo", 50, true, Some(tokens))
                .unwrap();
        }
        let p50 = collector
            .quantile_over_window(METRIC_SKILL_TOKENS, Duration::from_secs(3600), 0.5)
            .unwrap();
        // Median of 100..=700 step 100 is 400.
        assert_eq!(p50, Some(400.0));
    }

    #[test]
    fn fewer_than_min_samples_returns_none() {
        let collector = MetricsCollector::new().unwrap();
        // Only 3 samples — below MIN_BASELINE_SAMPLES = 5.
        for tokens in [100, 200, 300] {
            collector
                .record_skill_invocation("demo", 50, true, Some(tokens))
                .unwrap();
        }
        let result = collector
            .quantile_over_window(METRIC_SKILL_TOKENS, Duration::from_secs(3600), 0.5)
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn dyn_baseline_query_object_safe() {
        // Compile-time guard: BaselineQuery must remain object-safe
        // so callers can store it as Box<dyn BaselineQuery>.
        fn _assert_object_safe(_: &dyn BaselineQuery) {}
    }
}
