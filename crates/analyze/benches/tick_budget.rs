//! Tick-budget perf benchmark (TASK-024).
//!
//! Validates spec SC1: a tick on the standard fixture (200 skills /
//! 50 commands / 20 plugins / 3 MCPs) completes with median <50 ms
//! and p99 <200 ms. Wired into `make bench` (which runs
//! `cargo bench --workspace`).
//!
//! Criterion's default sample size (100) and confidence interval
//! (95 %) are sufficient for the SC1 thresholds; we don't override
//! them so the bench stays comparable across machines.

use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use skrills_analyze::cold_window::engine::TickInput;
use skrills_analyze::cold_window::ColdWindowEngine;
use skrills_test_utils::cold_window_fixtures::standard_snapshot;

/// SC1 median budget — assertion runs separately as a `#[test]`
/// (criterion benches don't fail the build on regression).
const _MEDIAN_BUDGET_MS: u128 = 50;
/// SC1 p99 budget.
const _P99_BUDGET_MS: u128 = 200;

fn bench_tick_standard(c: &mut Criterion) {
    let engine = ColdWindowEngine::with_defaults(100_000);
    let fixture = standard_snapshot();

    c.bench_function("cold_window_tick/standard_fixture", |b| {
        b.iter(|| {
            let input = TickInput::empty()
                .with_timestamp_ms(fixture.timestamp_ms)
                .with_token_ledger(fixture.token_ledger.clone())
                .with_load_sample(fixture.load_sample);
            engine.tick(input)
        });
    });
}

fn bench_tick_with_plugin_health(c: &mut Criterion) {
    // Plugin health collection is part of every real tick. Bench it
    // separately so SC1 numbers stay attributable.
    let engine = ColdWindowEngine::with_defaults(100_000);
    let fixture = standard_snapshot();

    c.bench_function("cold_window_tick/with_plugin_health", |b| {
        b.iter(|| {
            let input = TickInput::empty()
                .with_timestamp_ms(fixture.timestamp_ms)
                .with_token_ledger(fixture.token_ledger.clone())
                .with_plugin_health(fixture.plugin_health.clone())
                .with_load_sample(fixture.load_sample);
            engine.tick(input)
        });
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_millis(500))
        .measurement_time(Duration::from_secs(3));
    targets = bench_tick_standard, bench_tick_with_plugin_health
}
criterion_main!(benches);
