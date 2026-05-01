//! Adaptive cadence policy for the cold-window engine.
//!
//! Per `docs/archive/2026-04-26-cold-window-brief.md` § 4.2, the default
//! [`LoadAwareCadence`] implements the policy:
//!
//! ```text
//! recent edit (<10s)?  → tick = max(base / 2, min)
//! load_ratio > 0.9     → tick = min(base * 4, max)
//! load_ratio > 0.7     → tick = min(base * 2, max)
//! else                 → tick = base
//! ```
//!
//! The 0.7 / 0.9 thresholds borrow the Linux scheduler's
//! "moderately loaded" / "heavily loaded" classifications so operator
//! intuition transfers without inventing new tuning constants.
//! Power-of-two backoff multipliers compose with hysteresis without
//! oscillating at the threshold boundary.

use std::time::Duration;

use skrills_snapshot::LoadSample;

/// Strategy for deciding the duration until the next tick.
///
/// The default implementation is [`LoadAwareCadence`]; users
/// may override by implementing this trait and passing it to the
/// engine via the `with_strategies()` constructor.
pub trait CadenceStrategy: Send + Sync {
    /// Decide how long to wait before firing the next tick given
    /// the current load sample.
    fn next_tick(&self, sample: LoadSample) -> Duration;
}

/// Threshold below which a recent file edit accelerates the cadence
/// to keep the user's feedback loop tight.
pub const RECENT_EDIT_THRESHOLD_MS: u64 = 10_000;

/// Heavy-load ratio (loadavg / cores). At or above this, cadence
/// quadruples (capped at `max`).
pub const HEAVY_LOAD_THRESHOLD: f64 = 0.9;

/// Moderate-load ratio (loadavg / cores). At or above this, cadence
/// doubles (capped at `max`).
pub const MODERATE_LOAD_THRESHOLD: f64 = 0.7;

/// Default load-aware adaptive cadence.
///
/// Constructed with sensible defaults via [`LoadAwareCadence::new`];
/// configurable via the builder methods. All fields are public so
/// custom cadence strategies can compose with this one.
#[derive(Debug, Clone, Copy)]
pub struct LoadAwareCadence {
    /// Baseline tick duration when load is normal.
    pub base: Duration,
    /// Floor: cadence will never drop below this.
    pub min: Duration,
    /// Ceiling: cadence will never exceed this.
    pub max: Duration,
    /// CPU count used to normalize loadavg into a ratio.
    pub cores: usize,
}

impl LoadAwareCadence {
    /// Construct with the cold-window-spec defaults
    /// (`base = 2s`, `min = 500ms`, `max = 8s`, `cores = num_cpus::get()`).
    pub fn new() -> Self {
        Self {
            base: Duration::from_secs(2),
            min: Duration::from_millis(500),
            max: Duration::from_secs(8),
            cores: num_cpus::get(),
        }
    }

    /// Override the baseline tick duration.
    pub fn with_base(mut self, base: Duration) -> Self {
        self.base = base;
        self
    }

    /// Override the cadence floor.
    pub fn with_min(mut self, min: Duration) -> Self {
        self.min = min;
        self
    }

    /// Override the cadence ceiling.
    pub fn with_max(mut self, max: Duration) -> Self {
        self.max = max;
        self
    }

    /// Override the CPU count (useful for tests).
    pub fn with_cores(mut self, cores: usize) -> Self {
        self.cores = cores;
        self
    }
}

impl Default for LoadAwareCadence {
    fn default() -> Self {
        Self::new()
    }
}

impl CadenceStrategy for LoadAwareCadence {
    fn next_tick(&self, sample: LoadSample) -> Duration {
        // Recent edit takes priority — keep the feedback loop tight
        // even when the system is also under load.
        if let Some(age_ms) = sample.last_edit_age_ms {
            if age_ms < RECENT_EDIT_THRESHOLD_MS {
                return (self.base / 2).max(self.min);
            }
        }

        let cores = self.cores.max(1) as f64;
        let load_ratio = sample.loadavg_1min / cores;

        if load_ratio > HEAVY_LOAD_THRESHOLD {
            (self.base * 4).min(self.max)
        } else if load_ratio > MODERATE_LOAD_THRESHOLD {
            (self.base * 2).min(self.max)
        } else {
            self.base
        }
    }
}

/// Read 1-minute loadavg from `/proc/loadavg` on Linux.
///
/// Returns 0.0 on platforms without `/proc` or when the file cannot
/// be parsed; in that case the [`LoadAwareCadence`] degrades to
/// `base` cadence (load_ratio is 0.0). This is the documented
/// graceful fallback (spec § 8 / Assumption A2).
pub fn read_loadavg_1min() -> f64 {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/loadavg")
            .ok()
            .and_then(|s| s.split_whitespace().next().map(String::from))
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(loadavg: f64, edit_age: Option<u64>) -> LoadSample {
        LoadSample {
            loadavg_1min: loadavg,
            last_edit_age_ms: edit_age,
        }
    }

    fn cadence() -> LoadAwareCadence {
        LoadAwareCadence::new()
            .with_base(Duration::from_secs(2))
            .with_min(Duration::from_millis(500))
            .with_max(Duration::from_secs(8))
            .with_cores(4)
    }

    #[test]
    fn recent_edit_halves_base() {
        let c = cadence();
        let tick = c.next_tick(sample(0.0, Some(5_000)));
        assert_eq!(tick, Duration::from_secs(1));
    }

    #[test]
    fn recent_edit_with_high_load_still_speeds_up() {
        // Recent edit takes priority over load backoff.
        let c = cadence();
        let tick = c.next_tick(sample(8.0, Some(3_000)));
        assert_eq!(tick, Duration::from_secs(1));
    }

    #[test]
    fn old_edit_does_not_trigger_speedup() {
        let c = cadence();
        let tick = c.next_tick(sample(0.0, Some(60_000)));
        assert_eq!(tick, c.base);
    }

    #[test]
    fn no_edit_signal_uses_base_at_normal_load() {
        let c = cadence();
        let tick = c.next_tick(sample(0.5, None));
        assert_eq!(tick, c.base);
    }

    #[test]
    fn heavy_load_quadruples_capped_at_max() {
        let c = cadence();
        // load_ratio = 4.0 / 4 = 1.0, > 0.9 → base * 4 = 8s, capped at max = 8s.
        let tick = c.next_tick(sample(4.0, None));
        assert_eq!(tick, c.max);
    }

    #[test]
    fn moderate_load_doubles() {
        let c = cadence();
        // load_ratio = 3.2 / 4 = 0.8, > 0.7 → base * 2 = 4s.
        let tick = c.next_tick(sample(3.2, None));
        assert_eq!(tick, Duration::from_secs(4));
    }

    #[test]
    fn moderate_load_doubled_capped_at_max() {
        // base 5s, max 8s → 2*base = 10s would overflow, must cap at max.
        let c = LoadAwareCadence::new()
            .with_base(Duration::from_secs(5))
            .with_min(Duration::from_millis(500))
            .with_max(Duration::from_secs(8))
            .with_cores(4);
        let tick = c.next_tick(sample(3.2, None));
        assert_eq!(tick, Duration::from_secs(8));
    }

    #[test]
    fn next_tick_always_within_min_and_max() {
        // Property test: bounds must hold across a wide sample space.
        let c = cadence();
        let load_samples = [0.0, 0.1, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];
        let edit_ages = [None, Some(0), Some(5_000), Some(60_000), Some(u64::MAX)];
        for &load in &load_samples {
            for &age in &edit_ages {
                let tick = c.next_tick(sample(load, age));
                assert!(
                    tick >= c.min,
                    "tick {tick:?} below min {:?} for load {load} age {age:?}",
                    c.min
                );
                assert!(
                    tick <= c.max,
                    "tick {tick:?} above max {:?} for load {load} age {age:?}",
                    c.max
                );
            }
        }
    }

    #[test]
    fn cores_zero_is_treated_as_one() {
        // Defensive: cores=0 would divide by zero; we clamp to 1.
        let c = cadence().with_cores(0);
        let tick = c.next_tick(sample(0.5, None));
        // load_ratio = 0.5 / 1 = 0.5, normal → base
        assert_eq!(tick, c.base);
    }

    #[test]
    fn read_loadavg_returns_finite_number() {
        let v = read_loadavg_1min();
        assert!(v.is_finite());
        assert!(v >= 0.0);
    }
}
