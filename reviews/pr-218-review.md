# PR Review — #218 `feat(cold-window): v0.8.0 TUI + browser real-time dashboard`

**Branch**: `cold-window-analysis-0.8.0` → `master`
**Scale**: 150 files, +19,787 / −10,148, 44 commits
**Scope mode**: standard
**Reviewer**: `/sanctum:pr-review` (Claude Code)
**Date**: 2026-04-28

---

## Verdict: REQUEST CHANGES

The cold-window v0.8.0 feature is genuinely shipped — 31 plan tasks (T001–T031) trace cleanly to commits, version consistency is correct across all 14 crates, the parity test exists and is semantically meaningful, and the alert-hysteresis fix uses real consecutive-crossing semantics. However, six findings prevent merge:

1. **Proof-of-work claim is contradicted** — PR description says `cargo fmt --check` is clean; it fails on 12 files.
2. **`--research-rate` CLI flag silently does nothing** — accepted, parsed, never wired downstream.
3. **`research_quota` is permanently `None`** — TASK-011 acceptance criterion (persisted state, restored on startup) is implemented in `tome::dispatcher` but never connected to the running cold-window engine; the field exists only as a parity-test fixture.
4. **`kill_switch_engaged()` has no callers in sync paths** — Spec § 3.12 FR12 requires "subsequent sync operations refuse with a clear error message"; the predicate exists but no operational lock consumes it.
5. **TOCTOU race in `tome::dispatcher::try_dispatch`** — dedup check and bucket consume run under separate locks; SC10 capacity invariant is violable under concurrency.
6. **`AlertBand::high_clear` is stored but never evaluated for re-arm** — Spec § 3.4 mandates re-arm on `*_clear` re-cross; current implementation re-arms purely on dwell counter reset.

Items 2 and 3 are user-visible: a flag that does nothing and a status-bar indicator that always shows `None`.

---

## 1. Proof-of-Work Validation

| Claim | Result | Evidence |
|---|---|---|
| `cargo fmt --all -- --check` clean | **FAIL** | exit 1; 12 files need reformatting (see below) `[E1]` |
| `cargo clippy --workspace --all-targets -- -D warnings` clean | **PASS** | exit 0, 1m 07s `[E2]` |
| `cargo check --workspace --all-targets` | **PASS** | exit 0, 1m 02s `[E3]` |
| `cargo test --workspace` 2,343 / 2,343 passing | **NOT VERIFIED** | full suite skipped (>5min budget); spot-checks pass |
| `cargo build --workspace --release` 36.85s incremental | **NOT VERIFIED** | not re-run; check passed indicates compile path |

### `[E1]` fmt-check failure

```
12 files require formatting:
  crates/analyze/src/resolve/{graph.rs, mod.rs, resolver.rs}
  crates/discovery/src/types.rs
  crates/intelligence/src/usage/mod.rs
  crates/server/src/app/{mod.rs, skill_recommendations.rs}
  crates/sync/src/adapters/claude/{hooks.rs, mod.rs (×2), settings.rs}
  crates/test-utils/src/lib.rs
```

All 12 diffs are import-grouping / line-break stylistic differences — almost certainly the result of running rustfmt with a different toolchain version than the one the PR was committed under, or the in-branch refactor commits skipping local `cargo fmt` before push. The PR description claims `cargo fmt --all -- --check: clean`, which is provably false at HEAD.

This contradicts CLAUDE.md's pre-commit workflow contract:
> Always run quality checks BEFORE attempting to commit. If hooks fail, the code is not ready to commit.

---

## 2. Scope Assessment

### Spec-to-commit traceability — STRONG

Every one of TASK-001 through TASK-031 has a corresponding commit in the 44-commit sequence. Commit messages cite task IDs (e.g. `feat(analyze): cold-window engine integration GREEN (T006 + T008)`). The plan's "31 tasks, 85 points, 4 sprints" structure is honored.

### Scope creep — SIGNIFICANT (NON-BLOCKING)

11 of 44 commits are refactors not present in the spec/plan:

| Subsystem | Commits | Notes |
|---|---|---|
| `crates/sync` (claude adapter) | 5 commits, 23 files | platform_routing extraction, hooks/settings/mod splits, plugin_assets read split — entirely unrelated to cold-window |
| `crates/server` (beyond cold-window) | 3 commits | dispatcher, mcp_registry, recommend_skills extractions; cli/enums.rs split |
| `crates/intelligence` | 1 refactor + 1 perf | bail!→IntelligenceError migration (T2.7? — not in this plan); HashSet O(1) framework dedup |
| `crates/snapshot` | 1 refactor | cadence_label extraction |
| `tests/unit/*.py` | 7 new files | Python tests for plugin tooling — unrelated to cold-window |
| `tests/test-utils` | release_consistency invariants | Carryover from earlier release work |

These are all individually well-curated atomic commits. The concern is that bundling them into a feature release means a `git revert` of the merge to roll back cold-window also takes down 11 unrelated improvements. The PR description acknowledges RED-zone metrics and asserts "every line maps to spec/plan/evidence" — this is not accurate.

**Recommendation**: keep the scope-creep commits in this PR (the cherry-pick cost of separating now exceeds the benefit), but document it accurately: the PR is "v0.8.0 release branch" containing cold-window plus opportunistic cleanup, not a pure cold-window feature PR.

---

## 3. Blocking Findings

### B1 — `cargo fmt --check` fails; PR claims it's clean

See § 1. 12 files need reformatting. Action: `cargo fmt --all` and re-push.

### B2 — `--research-rate` CLI flag is parsed but never wired downstream

`crates/server/src/cold_window_cli.rs:54` declares `pub research_rate: u32` on `ColdWindowArgs`. The field is read in exactly two test assertions (`:386`, `:419`) and **nowhere else** in the entire codebase. It is never passed to the producer loop, the `ColdWindowEngine`, or the `tome::dispatcher`. Users running `skrills cold-window --research-rate 5` get no rate-limiting effect.

Spec § 3.10 lists `--research-rate <per-hour> (default 10)` as a binding configuration surface.

### B3 — `research_quota` is permanently `None` in the live cold-window

`crates/server/src/api/cold_window.rs:58` initializes `research_quota: None` on `ColdWindowDashboardState::new()` and never updates it. The field flows through `render_status_fragment` as `Option<(u32, u32)>`. The status bar always shows the placeholder when `None`. The `tome::dispatcher` correctly implements quota persistence with `BucketedBudget::persistent()` and `restart_exploit_quota_does_not_fully_reset` test, but the dispatcher is **not connected to the cold-window status-bar pipeline**. TASK-011 acceptance criterion (visible quota state, persisted across restart) is half-implemented.

### B4 — `kill_switch_engaged()` has no operational callers (FR12)

`crates/analyze/src/cold_window/alert.rs:95` defines `kill_switch_engaged() -> bool` as a pure predicate on `LayeredAlertPolicy`. `grep -rn "kill_switch"` returns hits only inside `alert.rs` itself (lines 95, 340, 347, 351–355). No sync path, no server path, no engine tick reads this predicate to refuse a mutating operation. Spec § 3.12 FR12 explicitly requires:

> If cumulative tokens exceed `--alert-budget` ceiling: A WARNING-tier alert fires with severity Critical. Subsequent sync operations refuse with a clear error message.

The alert-emission half is implemented; the operational-lock half is not. SC11 is half-met.

### B5 — TOCTOU race in `tome::dispatcher::try_dispatch`

`crates/tome/src/dispatcher.rs:176-217` performs (1) acquire `in_flight` lock → check dedup → release; (2) acquire `bucket` lock → consume token → drop; (3) re-acquire `in_flight` → record dispatch. Between (1) and (2), two concurrent callers with the same fingerprint can both pass the dedup check, both consume tokens, and both record. Under the `BucketedBudget` capacity invariant test (SC10), single-threaded passes; no concurrent stress test exists. Action: collapse the three critical sections into one, or use a single `Mutex` covering both maps.

### B6 — `AlertBand::high_clear` stored but unused for re-arm gate

`crates/analyze/src/cold_window/alert.rs:202-207` resets `entry.dwell_ticks = 0` when the condition leaves the band, but the `cleared` flag at line 205 and the `high_clear` value carried on the emitted `Alert` wire struct are write-only — `evaluate()` does not read `high_clear` to decide whether to suppress re-arm. Spec § 3.4 mandates `*_clear` re-cross before re-arming. The traits.rs:30-33 docstring agrees: "re-armed when the condition re-crosses the matching `*_clear` threshold." The implementation re-arms purely on dwell-counter reset. For a token total oscillating between 19K and 22K (below 19K only briefly), the implementation suppresses the alert permanently after the first drop below 20K — stricter than intended hysteresis but inconsistent with the documented band semantics.

---

## 4. Non-Blocking Findings (Should Fix Before Tag)

### N1 — `#[serde(tag = "kind")]` missing on snapshot enums (TASK-003 mandate)

All four enums in `crates/snapshot/src/types.rs` (`Severity`, `HintCategory`, `ResearchChannel`, `HealthStatus`) are unit-only and serialize as bare lowercase strings. Spec § 9 / TASK-003 requires `#[serde(tag = "kind")]` for proto3 `oneof`-compatibility. Today's wire format is fine; adding any payload variant in v0.9.0 is a silent wire-breaking change.

### N2 — `crates/snapshot/src/serde_impls.rs` does not exist (TASK-003 layout)

The plan explicitly enumerates this file. Missing. Either delete the requirement from the plan retrospective, or stub the file.

### N3 — SC7 chaos test passes vacuously for oscillating sequences

`crates/analyze/tests/chaos.rs:107-151` (`oscillating_chaos_meets_sc7_via_hysteresis`) asserts `max_per_hour < 12`. With current `min_dwell=2` and the dwell-reset on non-consecutive crossings, the oscillating signal fires zero alerts — `0 < 12` passes. The test would also pass for an over-suppression bug. Add a lower-bound assertion (`transitions >= 1`) on the monotonic chaos sequence so revert-tests are bidirectional.

### N4 — Latent SQL injection pattern in `metrics::collector::collect_metric_values`

`crates/metrics/src/collector.rs:158-164` uses `format!("SELECT CAST({col} AS REAL) FROM {tbl} WHERE ...")` with `{col}` and `{tbl}` interpolated as f-string fields, not bound params. Currently safe because the `match metric` closure maps caller input to a closed whitelist of literals. Pattern is fragile: any future arm that maps user input to a column/table name without sanitizing becomes injectable. Replace with a `MetricColumn` enum whose variants impl `as_str() -> &'static str`.

### N5 — `BucketedBudget::should_query` reads bucket without consuming

`crates/tome/src/dispatcher.rs:283` returns `bucket.available >= 1.0` but does not call `try_consume`. Callers using only the trait API get unbounded probing. Either consume on `should_query` or document that callers must follow up with `try_dispatch`.

### N6 — Corrupt `~/.skrills/research-quota.json` blocks daemon boot

`crates/tome/src/dispatcher.rs:139` propagates `serde_json::from_slice` errors via `?`. A corrupt file (e.g. half-written quota state from a SIGKILL'd previous run) makes `BucketedBudget::persistent()` return `Err` permanently until the file is manually deleted. Fall back to `PersistedBucket::full()` with a CAUTION-tier alert about the recovery.

### N7 — `persist_bucket` is non-atomic

`crates/tome/src/dispatcher.rs:312` calls `std::fs::write(path, bytes)` directly. A crash mid-write produces N6's corrupt-file scenario. Use `tempfile::NamedTempFile::persist` or `write tmp + rename` for atomic replacement.

### N8 — No final SSE "shutting down" event sent on graceful shutdown

`crates/server/src/api/cold_window.rs:98` breaks out of the SSE loop on `RecvError::Closed` without emitting a final named event. Connected browsers see an abrupt close and fall into `EventSource.onerror` showing "reconnecting". Emit `event: shutdown\ndata: {}\n\n` before breaking.

### N9 — Parity test misses `plugin_health` and `severity` label assertions

`crates/server/tests/cold_window_parity.rs:117` sets `plugin_health: vec![]` in the fixture; the test asserts no field for plugin health. Severity labels (`"caution"`, `"warning"`) are never compared between surfaces. A divergence in either surface would not fail the test.

### N10 — Round-trip serde test covers 2 of 12 enum variants

`crates/snapshot/src/lib.rs:118-124` round-trips a fixture using only `Severity::Warning` and `HealthStatus::Ok`. The other 10 variants (Caution/Advisory/Status, Warn/Error/Unknown, all `HintCategory`, all `ResearchChannel`) are unexercised. Add a parameterized round-trip test over all variants.

### N11 — Research-rate flag mis-wiring (covered in B2; non-blocking duplicate of fix scope)

---

## 5. Suggestions

- **S1**: `with_min_dwell(0)` silently behaves as `min_dwell=1`; document or reject.
- **S2**: Sibling subcommand smoke tests — only `cold-window` has `cli_dispatch_smoke.rs`. ~30 other subcommands rely on functional integration tests but have no dispatch guard. Consider a single parameterized smoke that walks `Commands` variants and asserts `--help` succeeds.
- **S3**: Bound the broadcast channel lag warning — currently the engine logs but does not surface lagging-subscriber drops as a STATUS alert (TASK-007 acceptance criterion).
- **S4**: The `ResearchBudget` trait re-export in `crates/snapshot/src/lib.rs:38-42` is a layering violation — `snapshot` is a wire-format crate, traits belong in `analyze` or a policy crate. Move the trait declaration.

---

## 6. Code Quality (pensive:code-refinement patterns)

- **Duplication**: low. `chaos_sequence`, `standard_snapshot`, `high_load_sample` are centralized in `crates/test-utils/src/cold_window_fixtures.rs` and consumed by `crates/analyze/tests/{chaos,cadence,tick_budget_floor,token_attribution}.rs`. Good single-source-of-truth for fixtures.
- **Test revert**:
  - **CLI dispatch (T031b)**: cli_dispatch_smoke.rs invokes the real `CARGO_BIN_EXE_skrills` binary with `["cold-window", "--help"]`. Reverting the `Commands::ColdWindow` arm trips clap's "unrecognized subcommand" and the test fails. **Real revert guard.**
  - **Alert hysteresis (T022)**: the monotonic `chaos_sequence` test fails on revert. The oscillating test passes vacuously (see N3). **Half guard.**
  - **Quota persistence**: `restart_exploit_quota_does_not_fully_reset` correctly fails on revert. **Real revert guard** — for the dispatcher; not for the cold-window integration (which is missing per B3).
- **Agent-curation signals**: minor. The 3 stale-comment fix commits and the 11 refactor commits suggest the branch was iterated heavily. No incomplete refactors found. Premature abstraction smell low — the trait surface (4 traits) maps 1:1 to spec § 6.

---

## 7. Positives

- **Documentation discipline**: spec, plan, brief, war-room, three tome research notes, user guide. Every functional requirement maps to at least one task; every success criterion to at least one verifier task. This is the strongest documentation-to-implementation traceability I've reviewed in this repo.
- **Version consistency**: 14 crates updated in lockstep to `0.8.0`. `docs/CHANGELOG.md`, `book/src/changelog.md`, `README.md` aligned. CHANGELOG dated `2026-04-28` matches today.
- **Slop discipline**: project's own `scripts/lint-prose-slop.sh` passes. No banned vocabulary in user-facing prose. Zero AI attribution in commits.
- **Conventional commits**: 44-commit log is clean — `feat`/`refactor`/`test`/`chore`/`docs`/`style`/`perf`/`fix` distribution is healthy.
- **TDD evidence**: TASK-005 RED phase, TASK-008 GREEN phase, hysteresis defect caught by property-based test (chaos.rs) — exactly the iron-law pattern.
- **CLI fix is a model in-branch defect-and-fix**: the CLI dispatch failure was caught by `make cold-window` dogfood, fixed in a focused commit (`7ac9d84`), and locked in by a real binary-level test (`8e87a92`).
- **HTTP transport, TUI, browser parity**: all three surface paths exist; the parity test compares semantic content from the same `Arc<WindowSnapshot>`.

---

## 8. Test Plan

Verification checklist for B1–B6 fixes:

- [ ] `cargo fmt --all` and re-push — verify `cargo fmt --all -- --check` exits 0
- [ ] Wire `args.research_rate` through to `BucketedBudget::new(rate, ...)` in `cold_window_cli::run()` — verify with a unit test that constructs args with `--research-rate 1` and asserts the dispatcher's bucket capacity is 1
- [ ] Wire `tome::dispatcher`'s persistent quota state to `ColdWindowDashboardState::research_quota` — verify with parity-test fixture using `Some((available, capacity))`
- [ ] Insert `kill_switch_engaged()` check at every sync mutation entry point (`sync::adapters::*::write_*`) — verify with integration test that token total > budget rejects writes
- [ ] Collapse TOCTOU race: replace separate `in_flight` and `bucket` locks with a single `Mutex<DispatcherInner>` — add a multi-threaded stress test (8 threads × 1000 distinct fingerprints) asserting `available` never goes negative
- [ ] Implement `*_clear` re-cross gate: when `entry.cleared`, only allow re-arm if current signal > `band.high_clear` — extend `chaos.rs` oscillating test with a lower-bound assertion (`transitions >= 1` for monotonic, exact bands for oscillating)

Additional (N-tier, before tag):

- [ ] Add `#[serde(tag = "kind")]` to the four snapshot enums; bump dependent fixture goldens
- [ ] Stub `crates/snapshot/src/serde_impls.rs` or remove from plan
- [ ] Replace `format!` SQL interpolation in `metrics::collector` with a `MetricColumn` enum
- [ ] Atomic `persist_bucket` (write tmp + rename); fall back to `PersistedBucket::full()` on corrupt file
- [ ] Emit final SSE `event: shutdown` on graceful close
- [ ] Round-trip serde test parameterized over all enum variants

---

## 9. Discussion / Out-of-Scope

The `attune:war-room-checkpoint` would auto-trigger on this PR (>3 blocking issues, architecture changes — new `skrills-snapshot` crate, new SSE surface). Recommended escalation: invoke war-room to deliberate on:

1. **Should this PR ship as v0.8.0** with the operational-lock half of FR12 deferred to v0.8.1, or block the tag until B4 is fixed?
2. **Quota persistence integration (B3)** — the dispatcher half is solid; was the UI integration intentionally deferred or accidentally cut?
3. **Refactor-bundling** — separate "v0.8.0 cold-window" from "v0.8.0 cleanup" in future releases?

These are judgment calls for the user, not blocking findings.

---

*Review generated by `/sanctum:pr-review`. Evidence: cargo fmt/clippy/check run locally on `cold-window-analysis-0.8.0`. Sub-agent reviews used `pensive:code-reviewer` for alert/CLI/snapshot/dispatcher domains. Insights candidates derived from B1–B6 + N1, N5, N9.*
