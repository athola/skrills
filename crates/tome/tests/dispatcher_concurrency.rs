//! Concurrency / robustness tests for `BucketedBudget`.
//!
//! Each test guards a specific PR-218 review finding:
//! - `concurrent_dispatch_never_oversells_capacity` — B5 (TOCTOU)
//! - `corrupt_persistence_file_recovers_at_full` — N6 (corrupt file)
//! - `persist_bucket_atomic_no_temp_leakage` — N7 (atomic write)
//! - `load_clamps_negative_available_to_zero` — NI5 (negative)
//! - `load_rejects_nan_available` — NI5 (NaN)
//!
//! NB5 (clock-warp) is asserted via the unit-test path in
//! `dispatcher.rs::tests` — see `clock_warp_does_not_saturate_bucket`.
//! Re-routing the system clock from an integration test would require
//! a public clock-provider trait, which exceeds the surface budget of
//! this fix; the in-crate unit test exercises the same code path
//! through the new `current_ms_checked` helper.

use std::sync::Arc;
use std::thread;

use skrills_snapshot::ResearchChannel;
use skrills_tome::dispatcher::{BucketedBudget, DispatchVerdict, PersistedBucket};

#[test]
fn concurrent_dispatch_never_oversells_capacity() {
    // B5: 8 threads, each issuing 1000 distinct fingerprints, racing
    // try_dispatch on the same budget. Capacity is small (50) so
    // contention is high. Invariants:
    //   1. Total Allowed verdicts <= rate_per_hour.
    //   2. `available` is never observed negative (final snapshot).
    let rate: u32 = 50;
    let budget = Arc::new(BucketedBudget::in_memory(rate));

    let n_threads = 8;
    let per_thread = 1000;

    let handles: Vec<_> = (0..n_threads)
        .map(|t| {
            let budget = Arc::clone(&budget);
            thread::spawn(move || {
                let mut allowed = 0usize;
                for i in 0..per_thread {
                    let fp = format!("t{t}-fp{i}");
                    if let DispatchVerdict::Allowed =
                        budget.try_dispatch(&fp, ResearchChannel::GitHub)
                    {
                        allowed += 1;
                    }
                }
                allowed
            })
        })
        .collect();

    let total_allowed: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();

    let state = budget.current_state();
    assert!(
        state.available() >= 0.0,
        "available went negative under contention: {}",
        state.available(),
    );
    assert!(
        total_allowed <= rate as usize,
        "oversold capacity: {total_allowed} > {rate} (capacity invariant SC10 violated)",
    );
}

#[test]
fn concurrent_dispatch_dedupes_same_fingerprint_under_race() {
    // B5 (sharper): N threads racing the SAME fingerprint. With the
    // pre-fix three-critical-section structure, multiple threads can
    // pass the dedup check, each consume a token, and each record —
    // resulting in more than one Allowed verdict for one fingerprint.
    // Expected: exactly one Allowed; the rest DuplicateInWindow.
    let rate: u32 = 1000; // generous, isolate dedup from quota
    let budget = Arc::new(BucketedBudget::in_memory(rate));

    let n_threads = 32;
    let barrier = Arc::new(std::sync::Barrier::new(n_threads));

    let handles: Vec<_> = (0..n_threads)
        .map(|_| {
            let budget = Arc::clone(&budget);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                budget.try_dispatch("shared-fp", ResearchChannel::GitHub)
            })
        })
        .collect();

    let verdicts: Vec<DispatchVerdict> = handles.into_iter().map(|h| h.join().unwrap()).collect();

    let allowed = verdicts
        .iter()
        .filter(|v| matches!(v, DispatchVerdict::Allowed))
        .count();
    assert_eq!(
        allowed, 1,
        "expected exactly one Allowed across {n_threads} racing threads, got {allowed}: {verdicts:?}",
    );
}

#[test]
fn corrupt_persistence_file_recovers_at_full() {
    // N6: a corrupt JSON file must NOT make persistent() return Err.
    // The daemon must boot with a fresh full bucket and log a warning.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("quota.json");
    std::fs::write(&path, b"{ this is :: not valid json ::: ").unwrap();

    let budget = BucketedBudget::persistent(7, path.clone())
        .expect("corrupt file must not block daemon boot");
    let state = budget.current_state();
    assert_eq!(state.rate_per_hour(), 7);
    // Full bucket on recovery (allow tiny refill jitter, but at most
    // the configured capacity).
    assert!(state.available() > 6.5 && state.available() <= 7.0);
}

#[test]
fn persist_bucket_atomic_no_temp_leakage() {
    // N7: after a successful dispatch we should see the final file
    // on disk, no `*.tmp.*` siblings, and the file should parse as
    // valid JSON. Temp files imply non-atomic write.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("quota.json");
    let budget = BucketedBudget::persistent(5, path.clone()).unwrap();
    let _ = budget.try_dispatch("fp", ResearchChannel::GitHub);

    let parent = path.parent().unwrap();
    let entries: Vec<_> = std::fs::read_dir(parent)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();

    let temps: Vec<_> = entries
        .iter()
        .filter(|n| n.contains(".tmp.") || n.ends_with(".tmp"))
        .collect();
    assert!(
        temps.is_empty(),
        "temp file leaked into persistence dir: {temps:?}",
    );

    let bytes = std::fs::read(&path).unwrap();
    let _: PersistedBucket =
        serde_json::from_slice(&bytes).expect("persisted file must be valid JSON");
}

#[test]
fn load_clamps_negative_available_to_zero() {
    // NI5: tampered file with negative `available` must not flow
    // through silently. We accept either "clamp to 0" or "fall back
    // to full bucket"; a negative or NaN value must NEVER survive.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("quota.json");
    std::fs::write(
        &path,
        br#"{"rate_per_hour":10,"available":-5.0,"last_refill_ms":0}"#,
    )
    .unwrap();

    let budget =
        BucketedBudget::persistent(10, path).expect("validation must not propagate as Err");
    let state = budget.current_state();
    assert!(state.available().is_finite());
    assert!(state.available() >= 0.0);
    assert!(state.available() <= state.rate_per_hour() as f64);
}

#[test]
fn load_rejects_nan_available() {
    // NI5: NaN poisons `min` (`NaN.min(x) == NaN`) and would propagate
    // forever through the bucket. Recover.
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("quota.json");
    // JSON spec disallows literal NaN; tampered files commonly emit
    // unquoted NaN. Ensure we recover even from non-conforming JSON.
    std::fs::write(
        &path,
        br#"{"rate_per_hour":10,"available":NaN,"last_refill_ms":0}"#,
    )
    .unwrap();

    let budget = BucketedBudget::persistent(10, path).expect("NaN must not block daemon boot");
    let state = budget.current_state();
    assert!(state.available().is_finite(), "NaN survived load");
    assert!(state.available() >= 0.0);
}
