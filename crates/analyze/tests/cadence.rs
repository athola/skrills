//! Adaptive cadence test (TASK-027).
//!
//! Validates spec SC12 ("cadence backs off under load") and the
//! brief § 4.2 policy diagram. Drives `LoadAwareCadence` directly
//! with synthetic `LoadSample` values pinned to a deterministic
//! 4-core baseline so behaviour is reproducible across machines
//! (test runners with 8+ cores would otherwise see different
//! load_ratio computations).

use std::time::Duration;

use skrills_analyze::cold_window::cadence::{
    CadenceStrategy, LoadAwareCadence, HEAVY_LOAD_THRESHOLD, MODERATE_LOAD_THRESHOLD,
    RECENT_EDIT_THRESHOLD_MS,
};
use skrills_snapshot::LoadSample;

/// 4-core baseline matches the high_load_sample / chaos fixtures.
const CORES: usize = 4;

fn cadence() -> LoadAwareCadence {
    LoadAwareCadence::new().with_cores(CORES)
}

#[test]
fn baseline_load_returns_base_tick() {
    // load_ratio = 0.4 / 4 = 0.1 — well below MODERATE_LOAD_THRESHOLD.
    let sample = LoadSample {
        loadavg_1min: 0.4,
        last_edit_age_ms: None,
    };
    assert_eq!(cadence().next_tick(sample), Duration::from_secs(2));
}

#[test]
fn moderate_load_doubles_tick() {
    // load_ratio = 3.0 / 4 = 0.75 ≥ 0.7 (MODERATE) but < 0.9 (HEAVY).
    let sample = LoadSample {
        loadavg_1min: 3.0,
        last_edit_age_ms: None,
    };
    let tick = cadence().next_tick(sample);
    assert_eq!(
        tick,
        Duration::from_secs(4),
        "moderate load (ratio 0.75) should double base tick"
    );
}

#[test]
fn heavy_load_quadruples_tick_capped_at_max() {
    // load_ratio = 4.0 / 4 = 1.0 ≥ 0.9 (HEAVY).
    let sample = LoadSample {
        loadavg_1min: 4.0,
        last_edit_age_ms: None,
    };
    let tick = cadence().next_tick(sample);
    // base * 4 = 8s, max = 8s — clamped equality.
    assert_eq!(tick, Duration::from_secs(8));
}

#[test]
fn heavy_load_clamps_at_max_even_with_higher_ratio() {
    // load_ratio = 16.0 / 4 = 4.0 — way above HEAVY threshold.
    let sample = LoadSample {
        loadavg_1min: 16.0,
        last_edit_age_ms: None,
    };
    let tick = cadence().next_tick(sample);
    assert_eq!(
        tick,
        Duration::from_secs(8),
        "max ceiling must clamp at 8s regardless of headroom"
    );
}

#[test]
fn recent_edit_halves_base_tick() {
    // Edit within last 10s and zero load → base / 2 = 1s.
    let sample = LoadSample {
        loadavg_1min: 0.1,
        last_edit_age_ms: Some(RECENT_EDIT_THRESHOLD_MS / 2),
    };
    assert_eq!(cadence().next_tick(sample), Duration::from_secs(1));
}

#[test]
fn recent_edit_clamps_at_min_floor() {
    // base / 2 = 1s, but min could clamp it. With min = 500ms,
    // 1s does not violate the floor; verify min is still enforced
    // by configuring an aggressively small base.
    let cadence = LoadAwareCadence::new()
        .with_cores(CORES)
        .with_base(Duration::from_millis(800));
    let sample = LoadSample {
        loadavg_1min: 0.1,
        last_edit_age_ms: Some(1_000),
    };
    let tick = cadence.next_tick(sample);
    // base / 2 = 400ms, but min floor is 500ms → tick = 500ms.
    assert_eq!(tick, Duration::from_millis(500));
}

#[test]
fn old_edit_does_not_accelerate() {
    // Edit older than RECENT_EDIT_THRESHOLD_MS → no acceleration.
    let sample = LoadSample {
        loadavg_1min: 0.1,
        last_edit_age_ms: Some(RECENT_EDIT_THRESHOLD_MS + 1_000),
    };
    let tick = cadence().next_tick(sample);
    assert_eq!(tick, Duration::from_secs(2), "stale edit ignored");
}

#[test]
fn recent_edit_takes_priority_over_load() {
    // The brief says recent-edit is the *first* check: a heavy
    // load with a recent edit still halves the cadence to keep
    // the developer feedback loop tight.
    let sample = LoadSample {
        loadavg_1min: 4.0,
        last_edit_age_ms: Some(2_000),
    };
    let tick = cadence().next_tick(sample);
    assert_eq!(
        tick,
        Duration::from_secs(1),
        "recent edit should beat heavy-load backoff"
    );
}

#[test]
fn moderate_threshold_is_strictly_exclusive_at_boundary() {
    // Brief § 4.2: `load_ratio > 0.7` (strict). Exactly at the
    // threshold falls through to base — verifies no spurious
    // doubling on a borderline-quiet system.
    let sample = LoadSample {
        loadavg_1min: MODERATE_LOAD_THRESHOLD * CORES as f64,
        last_edit_age_ms: None,
    };
    let tick = cadence().next_tick(sample);
    assert_eq!(tick, Duration::from_secs(2), "strict > at boundary");
}

#[test]
fn moderate_threshold_fires_just_above_boundary() {
    let sample = LoadSample {
        loadavg_1min: MODERATE_LOAD_THRESHOLD * CORES as f64 + 0.01,
        last_edit_age_ms: None,
    };
    assert_eq!(cadence().next_tick(sample), Duration::from_secs(4));
}

#[test]
fn heavy_threshold_is_strictly_exclusive_at_boundary() {
    let sample = LoadSample {
        loadavg_1min: HEAVY_LOAD_THRESHOLD * CORES as f64,
        last_edit_age_ms: None,
    };
    let tick = cadence().next_tick(sample);
    // Exactly at 0.9 → falls into the `> 0.7` branch (not `> 0.9`)
    // so we get the doubling, not quadrupling.
    assert_eq!(tick, Duration::from_secs(4), "strict > at heavy boundary");
}

#[test]
fn heavy_threshold_fires_just_above_boundary() {
    let sample = LoadSample {
        loadavg_1min: HEAVY_LOAD_THRESHOLD * CORES as f64 + 0.01,
        last_edit_age_ms: None,
    };
    assert_eq!(cadence().next_tick(sample), Duration::from_secs(8));
}

#[test]
fn synthetic_load_stream_walks_through_all_branches() {
    // Inject a synthetic LoadSample stream that walks through each
    // policy branch in order. Asserts the engine sees the expected
    // sequence of next_tick durations.
    let cadence = cadence();
    let stream: Vec<LoadSample> = vec![
        // baseline
        LoadSample {
            loadavg_1min: 0.0,
            last_edit_age_ms: None,
        },
        // moderate load
        LoadSample {
            loadavg_1min: 3.0,
            last_edit_age_ms: None,
        },
        // heavy load
        LoadSample {
            loadavg_1min: 4.0,
            last_edit_age_ms: None,
        },
        // recent edit (overrides load)
        LoadSample {
            loadavg_1min: 4.0,
            last_edit_age_ms: Some(1_000),
        },
        // edit ages out
        LoadSample {
            loadavg_1min: 0.1,
            last_edit_age_ms: Some(20_000),
        },
    ];
    let durations: Vec<Duration> = stream.into_iter().map(|s| cadence.next_tick(s)).collect();
    assert_eq!(
        durations,
        vec![
            Duration::from_secs(2), // baseline
            Duration::from_secs(4), // moderate
            Duration::from_secs(8), // heavy
            Duration::from_secs(1), // recent edit
            Duration::from_secs(2), // back to baseline
        ]
    );
}
