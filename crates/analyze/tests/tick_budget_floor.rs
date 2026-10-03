//! SC1 tick-budget regression floor.
//!
//! Criterion benches in `benches/tick_budget.rs` produce statistical
//! distribution data, but `cargo bench` runs lazily. This test
//! enforces a coarse-grained floor in `cargo test` so CI catches
//! catastrophic regressions even without a bench run:
//!
//! - Median tick over 50 iterations < 50 ms (SC1 median).
//! - 90th-percentile tick over 50 iterations < 200 ms. Not the max: one
//!   scheduler preemption on a loaded CI runner would fail a max-of-50
//!   gate with no code change, while a real regression moves the
//!   percentile too.
//!
//! The full distribution is in the criterion bench and reports
//! to `target/criterion/` for human review.

use std::time::Instant;

use skrills_analyze::cold_window::engine::TickInput;
use skrills_analyze::cold_window::ColdWindowEngine;
use skrills_test_utils::cold_window_fixtures::standard_snapshot;

const ITERATIONS: usize = 50;
const MEDIAN_BUDGET_MICROS: u128 = 50_000;
const P90_BUDGET_MICROS: u128 = 200_000;

#[test]
fn standard_fixture_tick_meets_sc1_floor() {
    let engine = ColdWindowEngine::with_defaults(100_000);
    let fixture = standard_snapshot();

    // Warmup: one tick to amortize allocator and first-use costs.
    let _ = engine.tick(
        TickInput::empty()
            .with_timestamp_ms(fixture.timestamp_ms)
            .with_token_ledger(fixture.token_ledger.clone())
            .with_load_sample(fixture.load_sample),
    );

    let mut samples: Vec<u128> = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let input = TickInput::empty()
            .with_timestamp_ms(fixture.timestamp_ms)
            .with_token_ledger(fixture.token_ledger.clone())
            .with_load_sample(fixture.load_sample);
        let start = Instant::now();
        let _ = engine.tick(input);
        samples.push(start.elapsed().as_micros());
    }
    samples.sort();

    let median = samples[ITERATIONS / 2];
    let p90 = samples[ITERATIONS * 9 / 10];
    let max = *samples.last().expect("non-empty");

    assert!(
        median < MEDIAN_BUDGET_MICROS,
        "SC1 median violated: median {median} us >= budget {MEDIAN_BUDGET_MICROS} us \
         (samples sorted: first={}, last={max})",
        samples[0]
    );
    assert!(
        p90 < P90_BUDGET_MICROS,
        "SC1 p90-of-{ITERATIONS} violated: p90 {p90} us >= budget {P90_BUDGET_MICROS} us \
         (max {max} us, reported only)"
    );
}
