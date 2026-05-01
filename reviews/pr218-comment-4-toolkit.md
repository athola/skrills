## Multi-Agent Toolkit Review — Wave 4

**Companion to**: [/sanctum:pr-review wave 4](https://github.com/athola/skrills/pull/218#issuecomment-4358018914)
**Agents dispatched**: 6 in parallel — comment-analyzer, pr-test-analyzer, silent-failure-hunter, type-design-analyzer, code-reviewer, code-simplifier
**Date**: 2026-05-01
**Method**: each agent briefed with wave-3 prior-art (`reviews/pr218-comment-3.md`) and asked to mark each prior finding FIXED / PARTIAL / STILL PRESENT, then surface NEW signal

---

### Headline

The toolkit independently corroborated **most wave-3 fixes** but found **4 new blocking findings + 10 important + ~11 suggestions**. Two patterns recur: wave-3 fixes that were applied at one level but not at the next (NB5 in dispatcher → not in CLI; NI9 at root → not at per-entry; NI15 in cursor → not in 3 other walkers). Plus two type-design escape hatches the wave-3 fixes left open (Deserialize bypassing `AlertBand::new`; `pub` fields on `PersistedBucket` bypassing `validated()`).

---

## New Blocking Findings (4)

### B1 — `cold_window_cli::now_ms` re-introduces the NB5 anti-pattern in production

`crates/server/src/cold_window_cli.rs:305-310`:

```rust
SystemTime::now().duration_since(UNIX_EPOCH).map(...).unwrap_or(0)
```

NB5 was explicitly fixed in `tome::dispatcher` (lines 489-503: `current_ms_checked() -> Option<u64>` + `bootstrap_ms` `u64::MAX` sentinel). The cold-window CLI producer wired its own duplicate clock without applying the fix. On NTP recovery / container time-warp / VM resume, the producer hands the engine `timestamp_ms = 0`; the engine's monotonic guards never see the fallible value because the producer fabricated `0` upstream. Cadence/dwell/`fired_at_ms` all degrade silently; alert hysteresis collapses.

**Fix**: delegate to `tome::dispatcher::current_ms_checked` (already pub via the same crate boundary) or replicate the `Option<u64>` plumbing; on `None`, skip the tick and `tracing::warn!` rather than fabricate `0`.

### B2 — `plugin_health::collect` silently drops per-entry I/O errors

`crates/analyze/src/cold_window/plugin_health.rs:137-138`:

```rust
entries.filter_map(Result::ok).filter(|e| e.file_type().map(...).unwrap_or(false))
```

NI9 (commit `717dbf4`) raised the *root* read failure to a CAUTION alert. The *per-entry* fan-out on the next two lines re-introduces the same hazard at the next level: a single plugin directory with restricted perms or a flapping NFS mount silently disappears from the health report. Operator sees "plugin healthy" when the plugin couldn't even be inspected.

**Fix**: same shape as wave-3's NI15 resolution — push each entry error and `file_type()` error onto `output.malformed`.

### B3 — `AlertBand` Deserialize bypasses `new()` validation

`crates/snapshot/src/types.rs:178-291`. NI4 fix made fields `pub(crate)` and added validating `AlertBand::new`, but **the derived `Deserialize`** still uses serde's field-by-field path which doesn't call `new`. The doc comment at `:252-256` explicitly acknowledges this trust assumption ("Deserialize ingests via the derived field-by-field path... round-trips of producer-emitted JSON") but the assumption is wrong: a v0.9.0 non-Rust producer or a tampered SSE payload can ship `{"low":100,"high":50,"low_clear":NaN,"high_clear":-1}` and downstream consumers (FieldwiseDiff, dashboard renderer at `cold_window.rs:501`) assume validity. Hot-path `new_unchecked` is `pub(crate)` and unused outside the snapshot crate — it's effectively dead code masking the real escape route.

**Fix**: manual `Deserialize` impl that calls `Self::new` and rejects on `BandError`. Mirror `serde_impls.rs` for unit enums.

### B4 — `PersistedBucket::validated` is private but all 3 fields are `pub`

`crates/tome/src/dispatcher.rs:71-130`. NI5 fix added `validated()` clamping NaN/Inf and bounds. But `rate_per_hour`, `available`, `last_refill_ms` are all `pub`. External code can construct `PersistedBucket { available: f64::NAN, rate_per_hour: 0, last_refill_ms: u64::MAX }` directly, then call `try_consume()` (always false on NaN compare) or feed it back through `BucketedBudget`. Validation runs only on the JSON-load path inside `persistent()`. `flush_persistence` and `current_state` clone the struct out by value, exposing `pub` mutation downstream.

**Fix**: flip fields to `pub(crate)` with `pub fn full()` + `pub fn new(rate, available, last_refill) -> Result<Self, _>` going through `validated()`.

---

## New Important Findings (10)

### I1 — URL-scheme XSS in research-pane SSE fragment

`crates/server/src/api/cold_window.rs:326` and escaper at `:425-438`. `<a href="{url}" ...>` where `url = html_escape(&f.url)`. The escaper handles `& < > " '` but does **not** validate the URI scheme. A `ResearchFinding.url = "javascript:alert(1)"` survives unchanged — quotes aren't required to break out of an `href` when the payload IS the attribute value. `target="_blank" rel="noopener"` doesn't block scheme execution. Findings come from external sources (HN/Reddit/Lobsters/GitHub) — a hostile post-title with a redirector link is the realistic vector.

**Fix**: gate scheme: `if f.url.starts_with("https://") || f.url.starts_with("http://") { html_escape(&f.url) } else { String::from("#") }`. Or `url::Url::parse` reject non-http(s).

### I2 — Per-tick deep clone of last snapshot under engine lock

`crates/analyze/src/cold_window/engine.rs:367`:

```rust
let prev = state.last_snapshot.as_deref().cloned()...
```

`as_deref()` derefs through `Arc<WindowSnapshot>` so `.cloned()` is a *deep clone* of the entire snapshot (alerts, hints, research findings, plugin_health). `prev` is then only used by reference in `is_alertable(&prev, ...)` (`:381`) and `evaluate(&prev, ...)` (`:388`) — the deep clone has no ownership purpose. `Arc::clone` would suffice. Lock window also includes the broadcast send at `:452`, extending hold time unnecessarily.

**Fix**: `let prev_arc = state.last_snapshot.clone();` then `let prev_ref = prev_arc.as_deref().unwrap_or(&empty_baseline);` with empty-baseline as a `Lazy`/static.

### I3 — `app/dispatcher.rs:300` discards `ensure_codex_skills_feature_enabled` result

```rust
let _ = crate::setup::ensure_codex_skills_feature_enabled(&home.join(".codex/config.toml"));
```

Returns `Result<()>` propagating `fs::read_to_string`/write errors. If `~/.codex/config.toml` is read-only or disk full, feature flag never lands but user sees "skills synced" (`:303`). Next codex skill fails opaquely.

**Fix**: at minimum `if let Err(e) = ... { tracing::warn!(error=%e, "could not ensure codex skills feature; users may need to re-run setup"); }`.

### I4 — NI15 anti-pattern recurs in 3 more walkers

Wave-3's NI15 fix only addressed `sync/adapters/cursor/mod.rs`. Same pattern in:

- `crates/server/src/app/dispatcher.rs:415` (sync-status skill counter)
- `crates/discovery/src/scanner.rs:302` (skill collector)
- `crates/discovery/src/scanner.rs:471` (agent collector)

Each `filter_map(|e| e.ok())` swallows `walkdir::Error` (loop detection, permission denied mid-walk, broken symlink, max-depth overrun). Single unreadable subdirectory under `~/.claude/skills` makes its sibling skills appear absent in counts; user sees "skills found in source: 14" when 17 exist. This propagates into the discovery layer that feeds attribution.

**Fix**: same warning pattern as NI15's resolution — `report.warnings.push` or `tracing::warn!` per dropped entry.

### I5 — `discovery/scanner.rs:184` saturation guard expresses coincidence not intent

```rust
vec![0u8; 1024.min(usize::try_from(size).unwrap_or(usize::MAX))]
```

Benign today (the `.min(1024)` clamps back), but exactly the saturation-guard hazard NB5 warned us against. One refactor reordering `1024.min(...)` is one OOM allocation away.

**Fix**: `usize::try_from(size).unwrap_or(1024).min(1024)` so the fallback expresses intent (cap at the prefix), not coincidence.

### I6 — `hint_pane::handle_pin_toggle` swallows pin-file I/O errors

`crates/dashboard/src/cold_window/hint_pane.rs:110`: `let _ = self.save();` discards `fs::create_dir_all`, `serde_json::to_vec_pretty`, `fs::write` errors. Doc-comment says "Best-effort persistence" but never surfaces the failure. User pins a hint with read-only `$HOME` (CI sandbox, locked container); UI succeeds, pin disappears next launch.

**Fix**: one-line `tracing::warn!` on Err. Behavior preserved.

### I7 — SSE `RecvError::Lagged` arm is untested

`crates/server/src/api/cold_window.rs:145-152` emits a styled "subscriber lagged by N ticks" status event when `tokio::sync::broadcast` reports lag. It's the only user-facing signal that a slow client missed snapshots. Existing tests (`cold_window_first_paint.rs`, `cold_window_parity.rs`, inline `sse_emits_shutdown_event_when_bus_closes`) cover only the `Ok` and `Closed` arms.

**Fix**: buffer-1 channel + sender that publishes >1 snapshot before receiver runs.

### I8 — SC10 `last_refill_ms` saturation path not directly asserted

`crates/tome/src/dispatcher.rs:39` documents that on missing/invalid `last_refill_ms` the bucket must saturate to capacity (not compute against zero). `clock_warp_does_not_saturate_bucket` (`:701`) covers the inverse; `validated_*` (`:747+`) covers `available` validation but not `last_refill_ms` specifically.

**Fix**: add `validated_rejects_future_last_refill_ms` and saturate-on-missing-timestamp tests.

### I9 — `MultiSignalScorer::with_*_weight` admits NaN

`crates/intelligence/src/cold_window_hints.rs:48-...`. `pub frequency_weight: f64` — `with_*` builders don't validate. NaN weight → NaN scores → `partial_cmp` returns `Equal`, corrupting hint ranking silently. Same NI6 anti-pattern as pre-fix `LayeredAlertPolicy`.

**Fix**: `try_with_*_weight(&self, w: f64) -> Result<Self, ScorerError>` rejecting NaN/negative.

### I10 — All-pub-fields anti-pattern across crate-internal types

Wire-format excuse holds for `WindowSnapshot` (proto3 round-trip). Does **not** hold for crate-internal types whose fields are mutated mid-flight or admit identity-zero instances:

- `Alert.fingerprint: String` (admits empty → AlertHistory dedupes everything)
- `Hint.impact / ease_score: f64`, `ScoredHint.score: f64` (doc says 0-10, unenforced)
- `TickInput` (`engine.rs:50`) — all pub, builders coexist with raw struct-literal mutation
- `ColdWindowDashboardState` (`api/cold_window.rs:71`) — `with_research_quota_source` clobbers but raw `state.bus = ...` bypasses
- `ColdWindowState` (`dashboard/state.rs:22`) — `pub bell_enabled` mutable mid-run, `pub master_ack_version: u64` allows external rewind
- `AlertHistory` / `AlertState` (`traits.rs:22,42`) — `pub fingerprints: HashMap`, `pub dwell_ticks` mutated by external policies

**Fix**: `pub(crate)` with constructors. Wave-3 NI4/NI5/NI6/NI7 set the precedent — extend to these types.

---

## Suggestions (highlights)

### S10 carry-over — TASK-NNN markers still in 12+ sites

Wave-3's `049670c` only covered `crates/analyze/src/cold_window/`. Also still present:

- `crates/dashboard/src/cold_window/{mod.rs:16,19,21,23, alert_pane.rs:1, hint_pane.rs:1, research_pane.rs:1, status_bar.rs:1, state.rs:35}`
- `crates/server/src/{cold_window_cli.rs:1,5, api/cold_window.rs:1}`
- `crates/snapshot/src/serde_impls.rs:5`
- `crates/tome/src/dispatcher.rs:359`
- `crates/analyze/src/cold_window/engine.rs:53,66` (short `T009`/`T011` form — wave-3's grep missed)

### Structural cleanups (~-97 LOC, all small)

- **F1** css_class/short_label as enum methods (-16 LOC) — wave-3 S1 follow-on, same pattern
- **F2** unify `AlertCounts` between dashboard and SSE renderer (-8 LOC)
- **F3** `SyncParams::for_kind(SyncKind, ...)` collapses 3 near-identical 20-LOC arms in `app/dispatcher.rs:161-260` (-40 LOC)
- **F4** `hint_pane::handle_key` digit-arm lookup table (-12 LOC)
- **F5** drop `category_label_pub` 1-line shim (-3 LOC) — has 1 use, fails CLAUDE.md's "no abstraction until 3rd use"
- **F6** collapse `try_with_*` / `with_*` builder pairs in `LayeredAlertPolicy` (-18 LOC)

### Test polish

- **TS-1** `nb2_three_consecutive_overruns_escalate_to_advisory` only checks endpoint severity. A regression firing Advisory on tick 1 would still pass. Assert per-tick sequence `[Status, Status, Advisory]`.
- **TS-2** `chaos.rs::oscillating_chaos_meets_sc7_via_hysteresis` is one-sided by design (author calls it out at `:150-156`). Lower-bound test uses *monotonic ramp*, not oscillating fixture. Add a "first crossing fires at least once before hysteresis engages" assertion.
- **TS-3** SC3 (TUI < 500ms) honest `#[ignore]` placeholder at `cold_window_first_paint.rs:88-101` — track TASK-024 in v0.8.1 backlog explicitly.

### Dead variants

- `DiscoveryError::Io` and `InvalidYaml` declared (`discovery/src/error.rs:14-22`) but `Io` is never constructed and `InvalidYaml` only at `types.rs:424` — without path context. Commit `952cb99` claimed "structured failure modes return DiscoveryError directly" but call sites still raise `anyhow!`. Either wire variants properly with `{ path, source }` or downgrade the commit's claim.

---

## Verified Wave-3 Fixes (independent corroboration)

| Wave-3 finding | Status | Evidence |
|---|---|---|
| C1 ResearchBudget docstring | FIXED | single source at `snapshot/types.rs:511` |
| NI11 IMPACT_WEIGHT rename | FIXED | `cold_window_hints.rs:35,38` |
| NI12 stale doc paths | FIXED | all `docs/cold-window-*` → `docs/archive/2026-04-26-cold-window-*` |
| NB5 dispatcher clock | FIXED | `current_ms_checked` returns `Option<u64>` (but B1: CLI re-introduced) |
| NI9 plugin-dir read | FIXED at root | (but B2: per-entry still drops) |
| NI13 SSE shutdown | FIXED | explicit `event: shutdown` at `api/cold_window.rs:153-159` |
| NI14 broadcast send | FIXED | throttled `subscriberless_ticks` counter at `engine.rs:452-466` |
| NI10 task panics | FIXED | three explicit arms at `cold_window_cli.rs:498-578` |
| NI15 cursor adapter | FIXED in cursor only | (but I4: 3 more walkers carry the pattern) |
| NI4 AlertBand | PARTIAL | (B3: Deserialize bypass) |
| NI5 PersistedBucket | PARTIAL | (B4: pub fields bypass) |
| NI6 LayeredAlertPolicy | FIXED | `try_with_*` returns `Result<Self, PolicyError>` |
| NI7 ResearchQuota newtype | FIXED at call sites | minor: `pub used/total` |
| NI8 HealthStatus default | FIXED | `#[default] Unknown` |
| S6 token-attribution | FIXED | full Vec equality at `token_attribution.rs:74-86` |
| S7 chaos tier assertions | FIXED | named fingerprints at `chaos.rs:108-117` |
| NB1/NB2 tick budget | FIXED | deterministic `SlowPolicy` harness, 3 revert tests |
| SC9 rolling baseline | FIXED | `alert.rs:782-854` covers warmup/post-warmup/eviction (wave-3 NI1 was incorrect) |
| 6 large refactors | PURE | inspected — kill-switch gates intact, no behavior drift |

---

## Strengths the toolkit corroborated

- `skrills-snapshot` remains a textbook wire-format crate; the recent `ResearchQuota` newtype (NI7) is colocated with its rationale (`types.rs:71-91`)
- WHY-comments at every non-obvious branch — `serde_impls.rs:1-30` is a model "why" comment (wire-compat trade-off, named failure mode of the alternative, points at consumer surface)
- Functional core / imperative shell holds in `engine.rs::tick` (one lock, broadcast send is the only side effect)
- Kill-switch gates remain at every mutating method in `claude/mod.rs:157,162,167,172,185,190,199`
- 6 large refactor commits (`a3ec92d`/`931c2c6`/`7e2b352`/`8f0522d`/`926ff92`/`2bb4460`) are pure — no behavior drift through the trait-dispatch reshuffles
- Commit hygiene clean across all 57 commits (no AI attribution, no emojis, no `--no-verify` traces, finding-tagged scopes)

---

## Recommended Action Plan

### Tier 1 — Block merge

1. CI blockers from `/sanctum:pr-review` wave-4 (NEW-B1 rustdoc, NEW-B2 Windows `tx`)
2. **B1** delegate `cold_window_cli::now_ms` to `current_ms_checked` (or replicate `Option<u64>` plumbing)
3. **B2** raise `plugin_health::collect` per-entry I/O errors via `output.malformed`
4. **B3** manual `AlertBand` Deserialize that calls `new()`
5. **B4** `PersistedBucket` fields → `pub(crate)`, add fallible `pub fn new`

### Tier 2 — Fix before tag v0.8.0

- **I1** URL scheme allowlist on research-pane `<a href>`
- **I2** Replace deep clone with `Arc::clone` in engine tick
- **I4** Apply NI15 fix to remaining 3 walkers
- **I9** `MultiSignalScorer::try_with_*_weight` returns `Result`
- **I7** Add `Lagged` arm test
- Either wire **NI16** `--skill-dirs` or remove the flag (carry-over)
- Decide on **DiscoveryError** — wire variants or trim them

### Tier 3 — File as v0.9.0 backlog

- I10 all-pub-fields tightening
- F1-F6 LOC reductions (~-97 LOC)
- TS-1, TS-2 test-tier assertion strengthening
- S10 TASK-NNN sweep across remaining 12 sites
- NI17/SC3 TUI integration test (TASK-024)

---

*Wave 4 toolkit review generated by `/pr-review-toolkit:review-pr` with 6 specialized agents (comment-analyzer, pr-test-analyzer, silent-failure-hunter, type-design-analyzer, code-reviewer, code-simplifier) running in parallel. Findings independently surfaced and aggregated; cross-referenced against wave-3 (`reviews/pr218-comment-3.md`) to mark each prior finding as FIXED, PARTIAL, or STILL PRESENT.*
