# Test Plan — PR #218

Verification checklist for the six BLOCKING findings in the review above:

### B1 — fmt-check
- [ ] Run `cargo fmt --all` locally
- [ ] Confirm `cargo fmt --all -- --check` exits 0
- [ ] Re-push the formatted commit

### B2 — `--research-rate` flag wiring
- [ ] In `crates/server/src/cold_window_cli::run()`, pass `args.research_rate` into `BucketedBudget::new(rate, ...)`
- [ ] Add a unit test: construct `ColdWindowArgs { research_rate: 1, .. }`, dispatch twice, assert second call returns `QuotaExhausted`
- [ ] Verify: `skrills cold-window --research-rate 1` reflects the configured rate in the status-bar quota indicator

### B3 — `research_quota` live integration
- [ ] Replace `research_quota: None` in `ColdWindowDashboardState::new()` with a value sourced from the `tome::dispatcher`'s `BucketedBudget` snapshot
- [ ] Add a regression test: drain the dispatcher to zero, render `/dashboard.sse`, assert the status-bar fragment shows `0/N` not the placeholder
- [ ] Manual smoke: `make cold-window`, observe quota indicator updates between `/dashboard` reloads

### B4 — `kill_switch_engaged()` operational lock
- [ ] Identify all sync mutation entry points (`crates/sync/src/adapters/*::write_*`, etc.)
- [ ] Insert `kill_switch_engaged()` precondition check, returning `Err(SyncError::TokenBudgetExceeded { .. })` when engaged
- [ ] Add integration test: build a fixture with `total_tokens > alert_budget`, assert sync ops fail with the expected error
- [ ] Update spec § 3.12 / FR12 acceptance to reference the test name

### B5 — TOCTOU in `tome::dispatcher::try_dispatch`
- [ ] Refactor: collapse `in_flight` and `bucket` into a single `Mutex<DispatcherInner>`
- [ ] Add stress test: 8 threads × 1000 distinct fingerprints; assert `available` never observed negative; assert total dispatches == capacity
- [ ] Re-run `restart_exploit_quota_does_not_fully_reset` — should still pass

### B6 — `*_clear` re-cross gate
- [ ] In `LayeredAlertPolicy::evaluate()`, add: `if entry.cleared && current_value > band.high_clear { entry.cleared = false; entry.dwell_ticks = 0; }`
- [ ] Extend `chaos.rs` oscillating test with assertion: signal oscillating between `high` and a value above `high_clear` should re-fire after dwell; signal oscillating between `high` and a value below `high_clear` should suppress
- [ ] Update traits.rs:30-33 docstring if implementation diverges from spec § 3.4

### Pre-merge gates (mandatory)

- [ ] `make format` clean
- [ ] `make lint` (clippy -D warnings) clean
- [ ] `make test --quiet` 100% pass; capture new test count (currently claimed 2,343)
- [ ] `make build` clean release
- [ ] `make cold-window` dogfood smoke succeeds (T029 acceptance)

### Post-merge / before-tag

- [ ] Address N1–N10 (see review § 4)
- [ ] Tag `v0.8.0` only after B1–B6 fixes and N1 (`#[serde(tag)]`) ship — N1 is wire-format-fragility risk for v0.9.0 gRPC
