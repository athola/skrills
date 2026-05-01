# PR #218 — Multi-Agent Toolkit Review

**Companion to**: prior `/sanctum:pr-review` ([review comment](https://github.com/athola/skrills/pull/218#issuecomment-4340280865))
**Agents dispatched**: 6 specialized reviewers, parallel — comment-analyzer, pr-test-analyzer, silent-failure-hunter, type-design-analyzer, code-reviewer (CLAUDE.md), code-simplifier
**Date**: 2026-04-28

---

## Summary

The toolkit caught **5 new blocking issues**, **17 new important issues**, and confirmed (via independent evidence trails) several findings from the prior `/sanctum:pr-review`. Three agents independently caught the **duplicate `ResearchBudget` trait** (one in `crates/snapshot`, one in `crates/analyze` — analyze copy is dead code); the test-analyzer caught **two unimplemented spec acceptance criteria** (SC9 rolling baselines, SC10 token-bucket enforcement) that the prior review missed.

**Updated verdict**: REQUEST CHANGES — at least 11 blocking findings (6 from prior review + 5 new).

---

## New Blocking Findings

### NB1 — Spec EC1 (tick-budget overrun) is unimplemented

`crates/analyze/src/cold_window/engine.rs:247-329` `tick()` does not measure its own elapsed time, has no consecutive-overrun counter, and never emits a STATUS alert. Spec § 5 EC1 requires "log a STATUS alert, delay next tick; after three consecutive overruns emit ADVISORY." None of this exists.

`crates/analyze/tests/tick_budget_floor.rs:25-63` measures wall-clock time **outside** the engine and asserts a budget — that's a test of the test fixture, not of the engine's overrun-handling. **Confirmed by silent-failure-hunter and pr-test-analyzer.**

### NB2 — `Severity::Status` is never constructed at runtime

A workspace-wide grep confirms `Severity::Status` exists only in:
- `crates/analyze/src/cold_window/alert.rs:165,174` — the `token-budget-status` template (statically defined)
- Renderer arms in `crates/dashboard/src/cold_window/state.rs` and `crates/server/src/api/cold_window.rs`

**No code path produces a STATUS-tier `Alert` at runtime.** TASK-007 acceptance criterion ("lagging subscribers drop, log a STATUS alert") and EC1 ("STATUS alert on tick overrun") both require dynamic STATUS-tier construction. Both are unimplemented.

### NB3 — Duplicate `ResearchBudget` trait — analyze copy is dead code

- `crates/snapshot/src/types.rs:287-296` — wire-format trait, byte-identical to:
- `crates/analyze/src/cold_window/traits.rs:88-97` — declares `BucketedBudget` as its default impl in the doc comment

But `crates/tome/src/dispatcher.rs:30,262` shows `BucketedBudget` impls `skrills_snapshot::ResearchBudget`, **not** the analyze copy. The analyze trait has zero impls in the workspace. The compile-time object-safety guard at `traits.rs:158` only proves the dead trait is object-safe.

A maintainer reading `traits.rs` will think the engine wires `BucketedBudget` through this trait. It does not. IDE jump-to-impl, doc cross-references, and future refactors will all hit this.

**Caught independently by comment-analyzer (C1), type-design-analyzer (#10), and code-reviewer (#1).**

### NB4 — `f40bf38` validate-watcher fix landed on dead code path

The commit message claims watcher errors are now visible. Reality:
- `crates/validate/src/watch.rs:85-90` — patched `collect_debounced_paths` (no live callers outside `tests`)
- `crates/validate/src/watch.rs:301` — `run_watch_loop` (line 219) calls `collect_debounced_paths_interruptible`, which still has the original `Ok(Err(_)) => {}` swallow

Production watch loop continues to silently drop watcher errors. The commit's user-facing claim ("flapping watchers visible to anyone running with RUST_LOG=...") is incorrect.

### NB5 — R10 quota mitigation defeated by `unwrap_or(0)` on `SystemTime`

`crates/tome/src/dispatcher.rs:316-321` uses `unwrap_or(0)` on `SystemTime::duration_since(UNIX_EPOCH)`. If the system clock briefly precedes UNIX_EPOCH (NTP recovery, container time-warp), `current_ms()` returns `0`. The next "real" call computes `elapsed_ms = now_ms.saturating_sub(0) ≈ 1.7T` → `bucket.refill` saturates to capacity, **undoing the entire R10 mitigation** that the file extensively documents and tests for. No log, no alert.

The pro-rata refill math was correct; the time-source masking subverts it.

---

## New Important Findings

### NI1 — SC9 (adaptive thresholds / rolling baseline) is unimplemented

Spec § 4.3 SC9: "rolling baselines within 5 minutes; pre-warmup falls back to constants." Workspace grep for `rolling_baseline`, `EmaState`, `allostatic` returns **no production code in `crates/analyze/`**. The thresholds in `LayeredAlertPolicy` (`alert.rs:48-61`) are static constants. SC9 is unimplemented and therefore vacuously untested. EC8 (pathological allostatic baseline) inherits the same root cause.

### NI2 — SC10 (`--research-rate` token-bucket invariant) is unenforced

The prior review (B2) found `--research-rate` accepted but unwired. The test-analyzer confirms this is **structurally** unenforceable: a workspace-wide grep returns **no `TokenBucket` or `BucketedBudget` instantiation in any production cold-window path**. The flag is purely cosmetic. `flush_persistence` (`tome/dispatcher.rs:253-259`) is documented as "called on graceful shutdown by TASK-031" but no caller exists.

### NI3 — Blocking I/O inside async producer task

`crates/server/src/cold_window_cli.rs:160-195` `producer_loop` is `async` and spawned on the multi-thread runtime, but every tick calls `plugin_collector.collect()` (`crates/analyze/src/cold_window/plugin_health.rs:111-147`) which does synchronous `std::fs::read_dir` + `std::fs::read_to_string` for every plugin's `health.toml`. Stalls a tokio worker for the duration of the walk. Wrap in `spawn_blocking` or use `tokio::fs`.

### NI4 — `AlertBand` admits illegal states

`crates/snapshot/src/types.rs:90-100` exposes `(low, low_clear, high, high_clear)` as four bare `pub f64`. Nothing prevents `low > high`, `low_clear > high_clear`, or `NaN`. Compounds B6 from the prior review: re-arm logic, when it lands, will read whatever a producer happened to write. Constructor-validated `AlertBand::new(...) -> Result<Self, BandError>` with `pub(crate)` fields would fix it.

### NI5 — `PersistedBucket::available` is NaN/negative-admitting

`crates/tome/src/dispatcher.rs:52-91` exposes `pub available: f64`. A deserialized bucket with `available = -5.0` or `f64::NAN` (file tampered, schema migration bug) silently flows through `min(rate as f64)` — `NaN.min(x) == NaN`. Validate on load (`persistent` at lines 135-148): clamp to `[0.0, rate_per_hour]`, reject NaN/Inf.

### NI6 — `LayeredAlertPolicy` exposes mutable invariant-bearing fields

`crates/analyze/src/cold_window/alert.rs:48-61` — every threshold is `pub`. `with_advisory_threshold(60_000).with_caution_threshold(40_000)` silently breaks tier ordering. Make fields `pub(crate)` and have `with_*` return `Result` (or add a `validate(self) -> Result<Self>` finalizer).

### NI7 — `Option<(u32, u32)>` for `research_quota` repeated 8+ times

`crates/server/src/api/cold_window.rs:49,80,110,277,311`, `crates/dashboard/src/cold_window/status_bar.rs:31,46,55,65`, parity test fixture. Tuple inversion (`(7,10)` vs `(10,7)`) is a silent bug magnet. Introduce `pub struct ResearchQuota { used: u32, total: u32 }` — eliminates parameter-swap class entirely. **Caught by both type-design-analyzer (#4) and code-simplifier (#1).**

### NI8 — `HealthStatus::Default = Ok` launders absence-of-data into positive health

`crates/snapshot/src/types.rs:231-243`: `#[default] Ok` means a freshly-constructed `PluginHealth` (via `..Default::default()` at `engine.rs:570-578`) reports "Ok" when nothing has been measured. `Unknown` is the correct sentinel. Either set `#[default] Unknown` or remove the `Default` derive.

### NI9 — Plugin-dir read failures masquerade as "no plugins"

`crates/analyze/src/cold_window/plugin_health.rs:113-122` — `read_dir` `Err(_) => return output` collapses "directory truly empty," "permission denied," "I/O error," and "broken symlink" into the same empty result. EC5 says malformed health files emit CAUTION; an unreadable plugins root produces no alert at all — every plugin silently disappears.

### NI10 — Producer/server task panics swallowed on shutdown

`crates/server/src/cold_window_cli.rs:139,141` — `let _ = producer_handle.await;` discards `JoinError`. A panicking producer that left the bus dead would look identical to a healthy shutdown.

### NI11 — `IMPACT_WEIGHT` is misnamed per its own comment

`crates/intelligence/src/cold_window_hints.rs:34-35`: `pub const IMPACT_WEIGHT` with doc "Weight applied to the **frequency** signal" — and line 92 confirms it multiplies `hint.frequency`. The companion `with_impact_weight` builder (line 68) extends the misnomer to public API. Commit `3cc4c6f` claimed to fix a stale HintScorer comment; this one slipped through. Rename to `FREQUENCY_WEIGHT` / `with_frequency_weight` (and `ACTIONABILITY_WEIGHT` → `IMPACT_WEIGHT` for symmetry, since it actually scales `hint.impact`).

### NI12 — Stale doc paths in 8+ rustdoc comments

`crates/snapshot/src/{lib.rs:9-10, types.rs:3,73,211,282-283}`, `crates/analyze/src/cold_window/{mod.rs:3, diff.rs:3, plugin_health.rs:3}`, `crates/dashboard/src/cold_window/mod.rs:7`, `crates/intelligence/src/cold_window_hints.rs:5`, `crates/server/tests/cold_window_parity.rs:12` reference `docs/cold-window-{brief,spec,plan}.md`. Reality: those files live under `docs/archive/2026-04-26-cold-window-{brief,spec,plan}.md`. Two files (`alert.rs:3`, `cadence.rs:3`) already use the archive path — proves the inconsistency. rustdoc cross-refs broken.

### NI13 — SSE shutdown event silent (companion to N8 from prior review)

`crates/server/src/api/cold_window.rs:98` breaks on `RecvError::Closed` with no farewell `event("status")`, no `tracing::info!`. Browser's `evt.onerror` handler shows "reconnecting…" forever, even though the server is intentionally gone.

### NI14 — `let _ = self.tx.send(...)` in `engine.tick`

`crates/analyze/src/cold_window/engine.rs:327` — `broadcast::send` returns `SendError` only with zero receivers. Permanent zero-receivers (every consumer crashed) is silent — the engine ticks forever into a dead bus. Worth a `tracing::debug!` rate-limited to once per N ticks.

### NI15 — Cursor adapter silently drops bad directory entries

`crates/sync/src/adapters/cursor/mod.rs:221` — `entries.filter_map(|e| e.ok())` swallows per-entry `io::Error`. Partial sync with no indication.

### NI16 — `ColdWindowArgs.skill_dirs` parsed but never consumed

`crates/server/src/cold_window_cli.rs` declares the field with doc "additional skill directories for the cold-window producer," but no consumer in `run` or `producer_loop` reads it. Pattern matches `--research-rate`: a documented flag that silently does nothing. Either wire it or remove it.

### NI17 — `SC2` (browser first paint < 1s) and `SC3` (TUI startup < 500ms) untested

Workspace grep for `first_paint`, `SC2`, `SC3` — zero hits. `cli_dispatch_smoke.rs:110` polls `/dashboard` for up to 5s — that's a "does it respond" check, not a sub-1s assertion. Add `Instant::now() ... assert_lt!(elapsed, Duration::from_millis(1000))` around the first GET.

---

## New Suggestions

### S1 — Hoist `severity_rank` / `channel_label` / `category_label` onto enums

Six duplicated mapping functions across TUI and SSE renderers (`dashboard/src/cold_window/{state.rs:161, alert_pane.rs:114, hint_pane.rs:274, research_pane.rs:216,226}`, `server/src/api/cold_window.rs:331,349,359`). Hoist to `Severity::short_label()`, `Severity::rank()`, `ResearchChannel::short_label()`, `HintCategory::label()` on the snapshot enums. ~−40 lines, single source of truth, eliminates TUI/SSE divergence risk.

### S2 — `WindowSnapshotBuilder` for tests

9+ test sites build `WindowSnapshot` via 12-field struct literals. `cold_window_fixtures::empty_snapshot()` exists but is bypassed. Add a `WindowSnapshotBuilder` with `with_total/with_alerts/with_load`. ~−120 lines, sharply lower friction when adding fields.

### S3 — `classify_token_total` band construction repeated 4×

`crates/analyze/src/cold_window/alert.rs:101-158` — 4 nearly-identical 7-line blocks. Extract `fn band(high: f64) -> AlertBand`. ~−20 lines, easier to maintain when re-arm gate (B6) lands.

### S4 — Newtype opportunities (low cost, prevents bug classes)

- `Fingerprint(String)` — used as `HashMap` key in `AlertHistory`, dispatcher, alerts; today any `&str` works
- `Score(f64)` — hint scores, currently raw `f64`
- `LoadRatio(f64)` — cadence-policy thresholds
- `BudgetCeiling(u64)` distinct from `TokenTotal(u64)` — would prevent the `kill_switch_engaged(token_total)` argument-swap bug class

### S5 — `tracing` migration of `println!`/`eprintln!` in dispatcher refactor

`crates/server/src/app/dispatcher.rs:87,92,173,192,211,232,234,255,257,283,299,328,349,370,374-377,407,409,427` — 20+ raw print sites. Pre-existing (carried verbatim from `app/mod.rs`), but commit `a3ec92d` was the right moment to migrate. Not a blocker — flagging because the refactor commit explicitly opted out of the larger 590-LOC `pub fn run` and 550-LOC `impl SkillService` splits, deferring them to follow-up PRs (good scope discipline elsewhere).

### S6 — Test quality: `token_attribution::tick_preserves_token_attribution`

`crates/analyze/tests/token_attribution.rs:71-86` only checks `len()` and `total`. If the engine reordered entries or renamed sources without changing counts, the test passes silently. Add a content-hash assertion on the per_skill/per_plugin/per_mcp slices.

### S7 — `chaos::monotonic_ramp_emits_at_most_one_alert_per_tier` has misleading assertion

`crates/analyze/tests/chaos.rs:84-104` asserts `fire_counts.len() >= 3`. A `len()` check on a HashMap counts distinct fingerprints, not per-tier firings. If a regression caused tier-Advisory to be skipped entirely, this still passes if the other tiers fire. Strengthen by asserting all expected tier fingerprints present **and** no individual count exceeds `TEN_MIN_TICKS - min_dwell`.

### S8 — `ratatui` workspace dep

Pinned at `0.30` independently in `crates/dashboard/Cargo.toml:24` and `crates/server/Cargo.toml:91` (with a justifying comment about avoiding double-resolution). Hoist to `[workspace.dependencies]`.

### S9 — `parity_test` 5x retry masks subscriber-attach race

`crates/server/tests/cold_window_parity.rs:243-246` sends the same snapshot 5x with sleeps. Faster CI machines where the subscriber attaches *after* all 5 sends would receive 0 events; the timeout path returns empty bytes, the `!browser_text.trim().is_empty()` guard fires the right error — the test would fail correctly but for the wrong reason. Delete the retry; the "subscribe before send" wait at lines 237-242 is correct and sufficient.

### S10 — Drop "(TASK-NNN GREEN phase)" suffix from rustdoc titles

`crates/analyze/src/cold_window/engine.rs:1`, `mod.rs:21-23`, `traits.rs:8-11` cite TASK-NNN as if the tasks are still in flight. Once shipped, this is transitional state. Migrate to ADR pointers since the spec already lives under `docs/archive/`.

---

## Type Design — Quantitative Ratings (`skrills_snapshot`)

| Type | Encapsulation | Invariants | Usefulness | Enforcement |
|---|---|---|---|---|
| `WindowSnapshot` | 4/5 | 3/5 (no `total`/`per_skill` consistency) | 5/5 | 3/5 |
| `Alert` | 3/5 | 2/5 (`Option<band>` semantics fuzzy) | 4/5 | 2/5 |
| `AlertBand` | 2/5 (4 raw `pub f64`) | 1/5 (no ordering) | 4/5 | 1/5 |
| `Severity` / `HintCategory` / `ResearchChannel` | 5/5 | 5/5 | 5/5 | 5/5 |
| `Hint` / `ScoredHint` | 4/5 | 3/5 (no 0-10 range check on impact/ease) | 4/5 | 2/5 |
| `TokenLedger` | 3/5 | 2/5 (`total` unverified) | 4/5 | 2/5 |
| `LoadSample` | 5/5 | 4/5 | 5/5 | 4/5 |
| `HealthStatus` / `HealthCheck` / `PluginHealth` | 4/5 | 2/5 (Default=Ok hides Unknown) | 4/5 | 2/5 |
| `PersistedBucket` | 2/5 (pub fields, NaN-admitting) | 2/5 | 4/5 | 2/5 |

---

## Strengths Confirmed by Multiple Agents

- **`skrills-snapshot` is a textbook wire-format crate** — zero back-edges to consumer crates, two dependencies (`serde` + `serde_json` dev), proto3-friendly invariants documented inline. Tagged-or-bare-string enums lock the wire shape via doctest + round-trip tests.
- **Functional core / imperative shell respected** in `ColdWindowEngine::tick` — strategy traits are pure, producer loop holds I/O.
- **HTML escape + DOMParser defense-in-depth** in `crates/server/src/api/cold_window.rs:18-23,374-387` — paired with XSS escaping tests at `:475-491`.
- **WHY-comments** in alert hysteresis (`alert.rs:194-201`), restart-exploit closure (`dispatcher.rs:128-134`), divide-by-zero guard in cadence (`cadence.rs:251`), and inverted-ease semantics in scorer (`cold_window_hints.rs:194-203`) — exactly the style CLAUDE.md asks for.
- **Sync adapter tests are behavioral, not fixture-bound** — copilot/cursor adapter tests at `crates/sync/src/adapters/{copilot,cursor}/tests.rs` cover round-trip, security-field preservation, path-traversal, broken symlinks, wrong-type fields.
- **`release_consistency.rs` walks actual workspace** — version drift between `Cargo.toml` files vs `plugin.json` would fail CI before shipping.
- **Refactor commits are real, not rearrangements** — `a3ec92d`/`931c2c6`/`7e2b352`/`8f0522d`/`926ff92`/`2bb4460` reduce file size, increase cohesion, and explicitly defer larger splits to follow-up PRs (good scope discipline).
- **Magic numbers are named with WHY**: `SNAPSHOT_CHANNEL_CAPACITY`, `ACTIVITY_RING_CAPACITY`, `RECENT_EDIT_THRESHOLD_MS`, `HEAVY_LOAD_THRESHOLD`, `MIN_TICK_MS`, `MIN_BASELINE_SAMPLES` — all `pub const` with rationales.

---

## Recommended Action Plan

### Tier 1 — Block merge (combined with prior review's B1–B6)

1. Delete the duplicate `ResearchBudget` trait in `analyze::cold_window::traits` (NB3); keep the `skrills_snapshot` one as the sole source
2. Implement EC1 tick-budget overrun: instrument `engine.tick()` wall-time, emit STATUS alerts (NB1, NB2)
3. Wire `producer_loop`'s `plugin_collector.collect()` through `spawn_blocking` (NI3)
4. Fix `f40bf38`'s real target: patch `collect_debounced_paths_interruptible` (NB4)
5. Replace `unwrap_or(0)` with explicit handling in `current_ms()` (NB5)
6. Plus prior B1–B6

### Tier 2 — Fix before tag

- Implement SC9 rolling baseline (or formally defer; spec § 4.3 currently lies)
- Connect `BucketedBudget` to live cold-window engine (resolves B2/B3 + NI2)
- `AlertBand`/`PersistedBucket`/`LayeredAlertPolicy` invariant enforcement (NI4-6)
- `ResearchQuota` newtype (NI7)
- `HealthStatus::Default` change (NI8)
- Plugin-dir read error → CAUTION alert (NI9)
- Rename `IMPACT_WEIGHT` (NI11)
- Stale doc-path bulk fix (NI12)
- `--skill-dirs` either wired or removed (NI16)
- Add SC2/SC3 first-paint assertions (NI17)

### Tier 3 — Test plan for the next review

- Run the silent-failure-hunter again on the same files after Tier 1 fixes
- Re-run pr-test-analyzer to confirm SC9/SC10 implementation status
- Stress test the dispatcher TOCTOU fix with 8 threads × 1000 fingerprints

---

*Review generated by `/pr-review-toolkit:review-pr`. Six specialized agents (comment-analyzer, pr-test-analyzer, silent-failure-hunter, type-design-analyzer, code-reviewer, code-simplifier) ran in parallel. Findings were aggregated and de-duplicated against the prior `/sanctum:pr-review` to highlight new signal.*
