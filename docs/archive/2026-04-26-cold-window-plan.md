# Cold-Window Real-Time Analysis: Implementation Plan

**Status**: Draft for review
**Date**: 2026-04-26
**Phase**: plan (attune mission Phase 3)
**Companions**: `docs/cold-window-brief.md` (architecture + research),
`docs/cold-window-spec.md` (functional requirements + contracts)

---

## 1. Architecture Snapshot

(Full detail in brief § 3; restated here for plan-side convenience.)

```
┌──────────────────────────────────────────────────────┐
│  ColdWindowEngine     (skrills-analyze::cold_window) │
│   tick(t) → Arc<WindowSnapshot>                       │
└──────────────────┬───────────────────────────────────┘
                   │
        ┌──────────▼──────────┐
        │  SnapshotBus         │   ← single source of truth
        │  broadcast<Arc<Snap>>│
        └──────────┬───────────┘
   ┌───────────────┼───────────────┐
   ▼               ▼               ▼
TUI Adapter    HTTP Adapter    Tome Worker
(dashboard)    (server SSE)    (quota-gated)
```

New crate: `skrills-snapshot` (wire format).
Modified crates: `skrills-analyze`, `skrills-intelligence`,
`skrills-metrics`, `skrills-tome`, `skrills-dashboard`, `skrills-server`,
`skrills-cli`.

---

## 2. File Structure

All paths relative to repo root.

| Path | Status | Owning Task |
|---|---|---|
| `Cargo.toml` (workspace) | edit | TASK-001, TASK-002 |
| `crates/snapshot/Cargo.toml` | create | TASK-001 |
| `crates/snapshot/src/lib.rs` | create | TASK-003 |
| `crates/snapshot/src/types.rs` | create | TASK-003 |
| `crates/snapshot/src/serde_impls.rs` | create | TASK-003 |
| `crates/analyze/src/cold_window/mod.rs` | create | TASK-007 |
| `crates/analyze/src/cold_window/engine.rs` | create | TASK-008 |
| `crates/analyze/src/cold_window/diff.rs` | create | TASK-014 |
| `crates/analyze/src/cold_window/alert.rs` | create | TASK-013 |
| `crates/analyze/src/cold_window/cadence.rs` | create | TASK-004 |
| `crates/analyze/src/cold_window/traits.rs` | create | TASK-005 |
| `crates/analyze/src/tokens.rs` | edit | TASK-009 |
| `crates/intelligence/src/recommend/scorer.rs` | edit | TASK-010 |
| `crates/intelligence/src/cold_window_hints.rs` | create | TASK-010 |
| `crates/metrics/src/collector.rs` | edit | TASK-012 |
| `crates/metrics/src/baseline.rs` | create | TASK-012 |
| `crates/tome/src/dispatcher.rs` | create | TASK-011 |
| `crates/tome/src/lib.rs` | edit | TASK-011 |
| `crates/dashboard/src/cold_window/mod.rs` | create | TASK-015 |
| `crates/dashboard/src/cold_window/alert_pane.rs` | create | TASK-015 |
| `crates/dashboard/src/cold_window/hint_pane.rs` | create | TASK-016 |
| `crates/dashboard/src/cold_window/research_pane.rs` | create | TASK-017 |
| `crates/dashboard/src/cold_window/status_bar.rs` | create | TASK-018 |
| `crates/dashboard/src/app.rs` | edit | TASK-015, TASK-018 |
| `crates/dashboard/src/events.rs` | edit | TASK-015 (master-ack key) |
| `crates/server/src/api/cold_window.rs` | create | TASK-019, TASK-020 |
| `crates/server/src/ui/cold_window.html` | create | TASK-019 |
| `crates/server/src/ui/fragments/alert_pane.askama` | create | TASK-020 |
| `crates/server/src/ui/fragments/hint_pane.askama` | create | TASK-020 |
| `crates/server/src/ui/fragments/research_pane.askama` | create | TASK-020 |
| `crates/server/src/ui/fragments/status_bar.askama` | create | TASK-020 |
| `crates/server/src/api/mod.rs` | edit | TASK-019 (route registration) |
| `crates/cli/src/dashboard.rs` | edit | TASK-021 |
| `crates/discovery/src/health.rs` | create | TASK-022 |
| `crates/discovery/src/lib.rs` | edit | TASK-022 |
| `crates/test-utils/src/fixtures/cold_window.rs` | create | TASK-006 |
| `crates/test-utils/tests/parity.rs` | create | TASK-023 |
| `crates/analyze/benches/tick_budget.rs` | create | TASK-024 |
| `crates/analyze/tests/chaos.rs` | create | TASK-025 |
| `crates/analyze/tests/token_attribution.rs` | create | TASK-026 |
| `crates/analyze/tests/cadence.rs` | create | TASK-027 |
| `book/src/cold-window.md` | create | TASK-028 |
| `book/src/SUMMARY.md` | edit | TASK-028 |
| `README.md` | edit | TASK-028 |
| `docs/CHANGELOG.md` | edit | TASK-028 |
| `Makefile` | edit | TASK-029 |

Total: 30 new files, 16 edits across 9 crates + workspace + docs.

---

## 3. Task Breakdown (30 tasks)

Story points use Fibonacci scale (1, 2, 3, 5, 8). `[P]` = parallelizable
within phase (no shared file edits).

### Phase 0 — Setup (2 pts)

#### TASK-001 — Scaffold `skrills-snapshot` workspace member
- **Phase**: 0
- **Points**: 1
- **Dependencies**: none
- **Files**: `crates/snapshot/Cargo.toml`, `crates/snapshot/src/lib.rs`
  (stub), `Cargo.toml` (workspace `members = [...]`)
- **Criteria**: `cargo build -p skrills-snapshot` succeeds with empty
  crate; workspace recognizes the new member.

#### TASK-002 — Workspace deps + version bump [P]
- **Phase**: 0
- **Points**: 1
- **Dependencies**: none
- **Files**: `Cargo.toml` (workspace.dependencies adds `num_cpus = "1"`),
  all `crates/*/Cargo.toml` (bump version to 0.8.0 in unison)
- **Criteria**: `cargo check --workspace` succeeds; `cargo metadata`
  shows version 0.8.0 across all members.

### Phase 1 — Foundation (15 pts)

#### TASK-003 — `WindowSnapshot` + wire-format types
- **Phase**: 1
- **Points**: 3
- **Dependencies**: TASK-001
- **Files**: `crates/snapshot/src/lib.rs`,
  `crates/snapshot/src/types.rs`, `crates/snapshot/src/serde_impls.rs`
- **Criteria**:
  - Types defined: `WindowSnapshot`, `Alert` (with 4-tier severity
    enum), `AlertBand`, `Hint`, `ScoredHint`, `ResearchFinding`,
    `TokenLedger`, `LoadSample`.
  - All types `#[derive(Clone, Serialize, Deserialize, Debug)]`.
  - All enums use tagged unions (`#[serde(tag = "kind")]`) — proto3
    `oneof`-compatible per spec § 9 (proto-friendly).
  - All `Option<T>` fields explicit, no `#[serde(default)]` shortcuts.
  - Round-trip serde test: `from_str(to_string(snap)) == snap`.

#### TASK-004 — `CadenceStrategy` trait + `LoadAwareCadence` default
- **Phase**: 1
- **Points**: 2
- **Dependencies**: TASK-003
- **Files**: `crates/analyze/src/cold_window/cadence.rs`
- **Criteria**:
  - Trait per spec § 6.2 with `next_tick(LoadSample) -> Duration`.
  - `LoadAwareCadence` implements the policy diagram in brief § 4.2:
    base / 2 on recent edit; \* 4 on load > 0.9; \* 2 on load > 0.7;
    base otherwise.
  - Defaults: `base = 2s`, `min = 500ms`, `max = 8s`.
  - Property test: `next_tick` always within `[min, max]`.
  - `loadavg` reader degrades gracefully on platforms without
    `/proc/loadavg` (returns 0.0; cadence falls back to base).

#### TASK-005 — `AlertPolicy` / `HintScorer` / `ResearchBudget` / `SnapshotDiff` trait declarations + RED tests
- **Phase**: 1
- **Points**: 5
- **Dependencies**: TASK-003
- **Files**: `crates/analyze/src/cold_window/traits.rs`,
  `crates/analyze/tests/contracts.rs`
- **Criteria**:
  - Four traits per spec § 6.1, 6.3, 6.4, 6.5 with documented
    default-impl pointers.
  - **TDD RED phase**: contract tests reference unimplemented default
    impls; tests COMPILE but FAIL with `unimplemented!()`. This is the
    Iron Law starting state.
  - `TodoWrite`: `proof:iron-law-red` for each trait.

#### TASK-006 — Test fixture builder [P]
- **Phase**: 1
- **Points**: 3
- **Dependencies**: TASK-003
- **Files**: `crates/test-utils/src/fixtures/cold_window.rs`
- **Criteria**:
  - `fixture::standard()` — 200 skills + 50 commands + 20 plugins +
    3 MCPs (used for SC1, SC4, SC5).
  - `fixture::chaos_sequence(n_ticks)` — synthetic mutation stream
    (used for SC7).
  - `fixture::high_load()` — loadavg-stressor harness (used for SC12).
  - Golden files for alert-sequence assertions.

#### TASK-007 — Snapshot bus + engine module skeleton [P]
- **Phase**: 1
- **Points**: 2
- **Dependencies**: TASK-003
- **Files**: `crates/analyze/src/cold_window/mod.rs`
- **Criteria**:
  - `pub struct ColdWindowEngine { tx: broadcast::Sender<Arc<WindowSnapshot>>, ... }`
  - **Bounded resources** (per R11): broadcast channel capacity = 16
    (lagging subscribers drop, log a STATUS alert); activity ring
    capped at last 100 entries.
  - `pub fn subscribe() -> broadcast::Receiver<Arc<WindowSnapshot>>`
  - `pub async fn run()` stub that loops on cadence and calls
    `tick()`.
  - Skeleton compiles; `tick()` returns a default snapshot.
  - Memory smoke test: 1000 ticks → resident set steady (no leak).

### Phase 2 — Core Implementation (21 pts)

#### TASK-008 — `ColdWindowEngine::tick()` integration (GREEN phase)
- **Phase**: 2
- **Points**: 5
- **Dependencies**: TASK-005, TASK-007, TASK-009 (token attribution),
  TASK-013 (alert policy), TASK-014 (diff)
- **Files**: `crates/analyze/src/cold_window/engine.rs`
- **Criteria**:
  - Re-walks discovery, computes token ledger, queries metrics
    baseline, runs hint scorer, runs diff, runs alert policy.
  - Wraps result in `Arc<WindowSnapshot>`, sends on bus.
  - **TDD GREEN phase**: contract tests from TASK-005 pass.
    `proof:iron-law-green` set per trait.
  - p99 tick under 200ms on `fixture::standard()` (SC1; verified by
    TASK-024 bench).

#### TASK-009 — Per-source token attribution [P]
- **Phase**: 2
- **Points**: 3
- **Dependencies**: TASK-003
- **Files**: `crates/analyze/src/tokens.rs`
- **Criteria**:
  - New fn `count_tokens_attributed(content, source: TokenSource)
    -> AttributedBreakdown` where `TokenSource ∈ {Skill, Plugin,
    Mcp, Conversation}`.
  - Attribution accuracy ≥95% on `fixture::standard()` (SC5;
    verified by TASK-026).
  - Existing `count_tokens()` API unchanged (additive).

#### TASK-010 — `MultiSignalScorer` for hints [P]
- **Phase**: 2
- **Points**: 3
- **Dependencies**: TASK-005
- **Files**: `crates/intelligence/src/cold_window_hints.rs`,
  `crates/intelligence/src/recommend/scorer.rs` (additive only)
- **Criteria**:
  - Implements `HintScorer` trait per spec § 6.3 formula.
  - Default weights: `IMPACT_WEIGHT = 2.0`,
    `ACTIONABILITY_WEIGHT = 1.5`, `HALF_LIFE_DAYS = 14`.
  - Contract test from TASK-005 passes.
  - Reuses existing `RecommendationScorer` machinery — additive only,
    no breaking change to its API.

#### TASK-011 — Tome dispatcher (group + dedup + inhibit + token-bucket) [P]
- **Phase**: 2
- **Points**: 5
- **Dependencies**: TASK-005
- **Files**: `crates/tome/src/dispatcher.rs`,
  `crates/tome/src/lib.rs` (re-export)
- **Criteria**:
  - `BucketedBudget` implements `ResearchBudget` trait per spec § 6.4.
  - `Dispatcher::dispatch(fingerprint)` groups within TTL, dedupes
    in-flight, inhibits redundant, respects token-bucket capacity.
  - Token-bucket invariant test: capacity not exceeded under stress
    (SC10).
  - TTL test: identical fingerprints within window collapse to one
    fetch.
  - **Quota persistence** (per R10): bucket state serialized to
    `~/.skrills/research-quota.json` on every successful dispatch and
    on graceful shutdown. On startup, bucket is restored and refilled
    pro-rata by elapsed wall-clock since last save (not reset).
    Restart-exploit test: rapid restart cycle does not bypass quota.

#### TASK-012 — Rolling-baseline query in `MetricsCollector` [P]
- **Phase**: 2
- **Points**: 3
- **Dependencies**: none (uses existing SQLite schema)
- **Files**: `crates/metrics/src/collector.rs`,
  `crates/metrics/src/baseline.rs`
- **Criteria**:
  - New fn `quantile_over_window(metric: &str, window: Duration,
    q: f64) -> Result<f64>`.
  - Underlying SQL uses existing time-series tables; no new schema.
  - Falls back to constants when window has insufficient data
    (warmup). Documented in spec § 8 (assumption A1).

#### TASK-013 — `LayeredAlertPolicy` (hysteresis + min-dwell + 4-tier)
- **Phase**: 2
- **Points**: 3
- **Dependencies**: TASK-005, TASK-012
- **Files**: `crates/analyze/src/cold_window/alert.rs`
- **Criteria**:
  - Implements `AlertPolicy` trait per spec § 6.1.
  - State machine handles: condition entering band, dwelling, firing,
    clearing, re-arming after `*_clear` re-cross.
  - Severity classification uses spec § 3.4 mapping.
  - Hard kill-switch fires deterministically at 100% budget (SC11).
  - Synthetic chaos test produces ≤12 alerts/hour (SC7;
    verified by TASK-025).

#### TASK-014 — `FieldwiseDiff`
- **Phase**: 2
- **Points**: 2
- **Dependencies**: TASK-005
- **Files**: `crates/analyze/src/cold_window/diff.rs`
- **Criteria**:
  - Implements `SnapshotDiff` trait per spec § 6.5.
  - Token tolerance ±2%; timestamps never alert; skill add/remove
    always alerts.
  - Contract test from TASK-005 passes.

### Phase 3 — Integration (21 pts)

#### TASK-015 — TUI alert pane + master-ack keystroke
- **Phase**: 3
- **Points**: 3
- **Dependencies**: TASK-008
- **Files**: `crates/dashboard/src/cold_window/alert_pane.rs`,
  `crates/dashboard/src/cold_window/mod.rs`,
  `crates/dashboard/src/app.rs` (add subscriber + pane mount),
  `crates/dashboard/src/events.rs` (add `Event::MasterAck` on `A` key)
- **Criteria**:
  - Renders 4-tier alert list sorted tier-then-recency.
  - WARNING rows red + bell on first appearance (subject to
    `--no-bell`).
  - Master-ack key `A` clears all CAUTION/ADVISORY/STATUS; WARNING
    requires per-row dismissal (SC8).
  - Subscriber receives `Arc<WindowSnapshot>` from bus.
  - **Resize handling** (per R9): on `Event::Resize`, panes redraw
    cleanly; no data loss; no panic. Test via crossterm's resize
    injection in headless TUI test.

#### TASK-016 — TUI hint pane [P]
- **Phase**: 3
- **Points**: 3
- **Dependencies**: TASK-008
- **Files**: `crates/dashboard/src/cold_window/hint_pane.rs`
- **Criteria**:
  - Ranked list per `ScoredHint::score` desc.
  - Filter keys: `1`/`2`/`3`/`4` for severity tier, `0` clears.
  - Pin key `P` toggles hint pin status; pinned hints stick on top.
  - Pin state persisted to `~/.skrills/cold-window-pins.json`.

#### TASK-017 — TUI research pane (collapsed default + badge) [P]
- **Phase**: 3
- **Points**: 2
- **Dependencies**: TASK-008, TASK-011
- **Files**: `crates/dashboard/src/cold_window/research_pane.rs`
- **Criteria**:
  - Collapsed by default; toggle key `R`.
  - Badge counter increments on new findings; never auto-expands
    (SC14, spec § 3.7 pull-only).
  - Research findings list shows source channel (GitHub / HN /
    paper / TRIZ), title, score, fetched-at timestamp.

#### TASK-018 — TUI status bar widget
- **Phase**: 3
- **Points**: 2
- **Dependencies**: TASK-004, TASK-015, TASK-016, TASK-017
- **Files**: `crates/dashboard/src/cold_window/status_bar.rs`,
  `crates/dashboard/src/app.rs` (mount the bar)
- **Criteria**:
  - Shows: tick rate (`2.0s [base]` / `4.0s [load 0.78]` /
    `1.0s [active edit]`), token budget (`68K / 100K`), alert
    counts per tier, research-quota remaining (SC13).
  - Updates every tick; no extra render overhead beyond ratatui's
    redraw.

#### TASK-019 — Server `/dashboard` HTML endpoint (HTTP/2)
- **Phase**: 3
- **Points**: 3
- **Dependencies**: TASK-008
- **Files**: `crates/server/src/api/cold_window.rs`,
  `crates/server/src/ui/cold_window.html`,
  `crates/server/src/api/mod.rs` (route registration),
  `crates/server/src/http_transport.rs` (HTTP/2 enabled)
- **Criteria**:
  - GET `/dashboard` returns initial page with empty panel slots
    and an `EventSource` script pointing at `/dashboard.sse`.
  - First-paint within 1s on `localhost` (SC2).
  - **HTTP/2 enabled** (per R8): server negotiates h2 over TLS
    (rustls already in workspace). Multi-tab test (5 tabs open
    simultaneously to `/dashboard`) all stay subscribed; HTTP/1.1
    per-origin 6-connection limit lifted by stream multiplexing.

#### TASK-020 — Server `/dashboard.sse` SSE endpoint + askama fragments
- **Phase**: 3
- **Points**: 5
- **Dependencies**: TASK-019
- **Files**: `crates/server/src/api/cold_window.rs` (extend),
  `crates/server/src/ui/fragments/*.askama` (4 templates)
- **Criteria**:
  - `axum::Sse` stream emits one named event per panel
    (`alert`, `hint`, `research`, `status`).
  - Each event payload is a pre-rendered HTML fragment via
    **askama** (compile-time templates, type-safe, zero-runtime
    dependency on a parser).
  - `Sse::keep_alive()` configured per spec EC4.
  - Browser parity test (TASK-023) shows semantic equivalence with
    TUI (SC4).

#### TASK-021 — CLI flags
- **Phase**: 3
- **Points**: 3
- **Dependencies**: TASK-004, TASK-008, TASK-013
- **Files**: `crates/cli/src/dashboard.rs`
- **Criteria**:
  - All flags from spec § 3.10 wired through to engine + cadence +
    dispatcher.
  - Config file `~/.skrills/cold-window.toml` supported; CLI overrides
    file values (spec § 3.10).
  - `--browser` starts the HTTP server alongside the TUI.

#### TASK-022 — Plugin participation via `health.toml` [P] [v0.8.0-stretch]
- **Phase**: 3
- **Points**: 3
- **Dependencies**: TASK-003
- **Priority**: **STRETCH for v0.8.0** — ship the convention but
  do not gate v0.8.0 release on community plugins adopting it. If
  Sprint 3 is over budget, defer this task to v0.9.0 where it
  ships alongside gRPC for the integrator persona (spec § 2.4).
- **Files**: `crates/discovery/src/health.rs`,
  `crates/discovery/src/lib.rs` (re-export)
- **Criteria**:
  - Discovery walker finds `<plugin>/health.toml` files and parses
    them into `HealthReport` per spec § 3.11.
  - Malformed `health.toml` emits CAUTION alert (spec EC5);
    plugin excluded from snapshot.
  - Reports flow into `WindowSnapshot::plugin_health` (added in
    TASK-003 if missing).
  - Test: synthetic plugin tree with one valid + one malformed
    `health.toml` produces correct snapshot + alert.

### Phase 4 — Polish (16 pts)

#### TASK-023 — Parity test fixture (TUI vs browser)
- **Phase**: 4
- **Points**: 3
- **Dependencies**: TASK-018, TASK-020
- **Files**: `crates/test-utils/tests/parity.rs`
- **Criteria**:
  - Headless TUI render → snapshot of pane text.
  - Headless browser render via `reqwest` GET `/dashboard.sse` → parse
    HTML fragments → text content.
  - Assertion: semantic equivalence on all displayed fields (SC4).

#### TASK-024 — Tick-budget perf benchmark [P]
- **Phase**: 4
- **Points**: 3
- **Dependencies**: TASK-008
- **Files**: `crates/analyze/benches/tick_budget.rs`
- **Criteria**:
  - Criterion bench on `fixture::standard()`.
  - Asserts median <50ms, p99 <200ms (SC1).
  - Wired into `make bench` target.

#### TASK-025 — Synthetic chaos test for alert budget [P]
- **Phase**: 4
- **Points**: 3
- **Dependencies**: TASK-013
- **Files**: `crates/analyze/tests/chaos.rs`
- **Criteria**:
  - 10-minute synthetic mutation stream from
    `fixture::chaos_sequence`.
  - Counts user-visible alerts after hysteresis + min-dwell + tier
    filtering.
  - Asserts <12/hr (SC7); golden files for alert sequence.

#### TASK-026 — Token attribution accuracy test [P]
- **Phase**: 4
- **Points**: 2
- **Dependencies**: TASK-009
- **Files**: `crates/analyze/tests/token_attribution.rs`
- **Criteria**:
  - Run on `fixture::standard()`.
  - Asserts ≥95% attribution accuracy vs ground truth (SC5).

#### TASK-027 — Adaptive cadence test (loadavg) [P]
- **Phase**: 4
- **Points**: 2
- **Dependencies**: TASK-004
- **Files**: `crates/analyze/tests/cadence.rs`
- **Criteria**:
  - Inject synthetic `LoadSample` stream.
  - Asserts cadence doubles when ratio > 0.7 (SC12) and quadruples
    when > 0.9.
  - Asserts halves on recent-edit signal (within `min` floor).

#### TASK-028 — Documentation refresh
- **Phase**: 4
- **Points**: 3
- **Dependencies**: TASK-001 through TASK-027 (i.e., implementation
  complete)
- **Files**: `book/src/cold-window.md`, `book/src/SUMMARY.md`,
  `README.md`, `docs/CHANGELOG.md`
- **Criteria**:
  - `book/src/cold-window.md` covers user-facing flows (the four
    spec personas), CLI flags, screenshots / GIFs.
  - `README.md` lifts a 3-line description + screenshot to top.
  - `docs/CHANGELOG.md` 0.8.0 entry lists every FR shipped.
  - `make book` + markdown lint passes.

#### TASK-029 — Makefile dogfood target [P]
- **Phase**: 4
- **Points**: 1
- **Dependencies**: TASK-021
- **Files**: `Makefile`
- **Criteria**:
  - `make cold-window` builds and launches `skrills dashboard --watch
    --browser` against the repo's own plugins/skills.
  - Documented in `book/src/development.md`.

#### TASK-030 — Final verification sweep
- **Phase**: 4
- **Points**: 1
- **Dependencies**: all preceding
- **Files**: none (pure verification)
- **Criteria**:
  - `make format` clean.
  - `make lint` clean (workspace-wide ruff/clippy/mypy/etc).
  - `make test --quiet` 100% pass on all crates.
  - `make build` clean release build.
  - Captures **proof-of-work** evidence `[E1]`–`[E5]` in commit msg.

#### TASK-031 — Graceful shutdown + signal handling [post-war-room addition]
- **Phase**: 3 (logically belongs with surfaces)
- **Points**: 2
- **Dependencies**: TASK-008, TASK-021
- **Files**: `crates/cli/src/dashboard.rs` (extend),
  `crates/analyze/src/cold_window/mod.rs` (extend),
  `crates/dashboard/src/app.rs` (extend)
- **Criteria**:
  - `Ctrl-C` / SIGTERM triggers a single-pass graceful shutdown:
    flush pending `MetricsCollector` writes, persist research
    quota state (per TASK-011), close SSE streams cleanly with a
    "shutting down" final event, drain broadcast channel, exit 0.
  - Shutdown completes within 2 seconds; if it doesn't, SIGKILL
    fallback after grace period.
  - Test: send `kill -INT` to a running daemon, verify clean exit
    - persisted quota state.
  - User-facing: `make cold-window` documents the kill mechanism.

---

## 4. Sprint Plan

| Sprint | Phase | Tasks | Points | Goal |
|---|---|---|---|---|
| 1 | 0 + 1 | T001–T007 | 17 | Foundation: workspace + wire format + fixtures + RED tests |
| 2 | 2 | T008–T014 | 24 | Core: engine + subsystems + GREEN tests |
| 3 | 3 | T015–T022, T031 | 26 | Surfaces: TUI + browser + CLI + graceful shutdown |
| 4 | 4 | T023–T030 | 18 | Polish: parity + perf + chaos + docs |
| **Total** | | **31** | **85** | |

Sprint capacity assumed at 18–26 points per sprint for one developer
(approx. 1 sprint = 1 working week). **Realistic wall-clock estimate
per RT-1 challenge: 5–6 weeks**, not 4 — accounts for review cycles,
revision rounds, and defect remediation. Tasks marked `[P]` within a
sprint can be parallelized across multiple branches.

---

## 5. Critical Path

The longest dependency chain that must run sequentially:

```
T001 → T003 → T005 → T008 → T015 → T018 → T023 → T030
 1pt   3pt    5pt    5pt    3pt    2pt    3pt    1pt   = 23 pts
```

**Critical-path discipline (per war-room amendment)**: TASK-005 trait
surface is **frozen at sprint-1 close**. Any post-freeze trait-shape
revisions roll into TASK-008 work, not back into TASK-005. This
prevents a sprint-1 trait revision from cascading through the entire
critical path.

All other tasks fan out from this spine:
- T009, T010, T011, T012 fan out after T005.
- T016, T017 fan out after T008.
- T019, T020, T021, T022 fan out after T008.
- T024–T029 fan out after their respective sources.

Single-developer execution: ~23 critical-path points = ~3–4 weeks of
focused work, with parallel tasks compressing wall-clock further if
multiple branches are used.

---

## 6. FR / SC Coverage

Every functional requirement and success criterion in the spec is
covered by at least one task.

### FR coverage

| FR | Tasks |
|---|---|
| FR1 — tick lifecycle | T008 |
| FR2 — single snapshot | T003, T007 |
| FR3 — token ledger | T009, T026 |
| FR4 — 4-tier alert model | T013, T015 |
| FR5 — master-acknowledge | T015 |
| FR6 — hint pane | T010, T016 |
| FR7 — research panel pull-only | T011, T017 |
| FR8 — browser surface (SSE+HTML) | T019, T020 |
| FR9 — TUI surface | T015, T016, T017, T018 |
| FR10 — configuration | T021 |
| FR11 — plugin participation | T022 |
| FR12 — hard kill-switch | T013, T021 |

### SC coverage

| SC | Verifier task |
|---|---|
| SC1 — tick budget | T024 |
| SC2 — browser first paint | T019 (tested in T023) |
| SC3 — TUI startup | T024 |
| SC4 — TUI/browser parity | T023 |
| SC5 — token attribution | T026 |
| SC6 — cold rewalk catches mutation | T008 (manual + T023) |
| SC7 — alert hygiene <12/hr | T025 |
| SC8 — master-ack | T015 |
| SC9 — adaptive thresholds warmup | T012 (in T013 tests) |
| SC10 — research quota | T011 (token-bucket invariant test) |
| SC11 — kill-switch deterministic | T013 |
| SC12 — adaptive cadence backoff | T027 |
| SC13 — status bar visibility | T018 |
| SC14 — research panel pull-only | T017 |
| SC15 — plugin participation tick | T022 |

---

## 7. Risks + Mitigations

| ID | Risk | Mitigation Task |
|---|---|---|
| R1 | Tick budget exceeded on >500 skill ecosystems | T009 (per-widget freshness) + T024 (bench) + T027 (adaptive cadence) |
| R2 | SSE keepalive drops behind NATs/proxies | T020 (`Sse::keep_alive`) + spec EC4 + T023 (parity test catches missed events) |
| R3 | Token estimation imprecision (heuristic); SC5 95% claim is fixture-bound, not field-validated | Documented in spec A1; consistent within snapshot; mitigated by ≥95% accuracy bar in T026; honest fixture-bound phrasing |
| R4 | Allostatic baseline pathological growth | T012 (warmup fallback) + spec EC8 |
| R5 | Tome external API rate-limited | T011 (token-bucket + dispatcher dedup) + spec EC3 |
| R6 | gRPC follow-up breaks v0.8.0 contract | T003 (proto-friendly types) — pure-additive in v0.9.0 |
| R7 | Plugin author ships malformed `health.toml` | T022 (spec EC5: CAUTION alert + exclude) |
| **R8** | **Browser HTTP/1.1 6-connection-per-origin limit blocks multi-tab** | **T019 promotion to HTTP/2 (rustls already in tree)** |
| **R9** | **Terminal resize during render → garbled UI / panic** | **T015 explicit resize-event acceptance criterion + headless test** |
| **R10** | **Token bucket reset by daemon restart → quota bypass** | **T011 persistence to `~/.skrills/research-quota.json` with pro-rata refill** |
| **R11** | **Long-running daemon memory growth (broadcast / activity feed unbounded)** | **T007 bounded broadcast (cap 16) + bounded activity ring (last 100)** |

No RED-tier tasks per `leyline:risk-classification`. Three YELLOW-tier
tasks (T008 engine integration, T020 SSE+fragments, T031 graceful
shutdown) — these need extra review attention; war-room cleared all
three this round.

---

## 8. Verification Commands

Per CLAUDE.md pre-commit workflow:

```bash
make format && make lint && make test --quiet && make build
```

Per-task verification examples:
- TASK-008: `cargo test -p skrills-analyze cold_window::engine`
- TASK-013: `cargo test -p skrills-analyze cold_window::alert -- --nocapture`
- TASK-020: `cargo test -p skrills-server api::cold_window`
- TASK-023: `cargo test -p skrills-test-utils --test parity`
- TASK-024: `cargo bench -p skrills-analyze tick_budget`
- TASK-025: `cargo test -p skrills-analyze --test chaos -- --include-ignored`

Bench thresholds enforced via `cargo-criterion` config; failure
breaks CI.

---

## 9. Open Items

- [ ] Plan-review gate: section-by-section approval per
  `attune:mission-orchestrator` plan-review module (architecture,
  then phases). Forced before execute phase.
- [ ] War-room sign-off on YELLOW tasks (T008, T020) if reviewer flags
  them.
- [ ] Confirmation that this plan supersedes any prior work in
  `crates/test-utils/tests/release_consistency.rs` (recent commits)
  is additive only (no overlap expected).

---
