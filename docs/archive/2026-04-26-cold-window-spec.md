# Cold-Window Real-Time Analysis: Specification

**Status**: Draft for review
**Date**: 2026-04-26
**Phase**: specify (attune mission Phase 2)
**Companion**: `docs/cold-window-brief.md` (architecture + research)

---

## 1. Overview / Context

### Problem

Users running skrills accumulate dozens of plugins, hundreds of skills, and
multiple MCP servers over time. They cannot see, in any continuous way:

- Which skills are wasting tokens (the largest invisible cost driver in
  long-running AI sessions, often 50%+ of system-prompt overhead).
- When a configuration change has degraded validation, broken a sync, or
  introduced redundant capabilities.
- What other practitioners have learned about similar problems — research
  must be hunted down manually.
- How their configuration compares to baselines they cannot articulate
  ("is 50K of MCP overhead too much?").

The existing skrills `dashboard` and `intelligence` crates expose snapshots
of state, but only on demand. The user has no continuous awareness — by
the time they re-run the dashboard, the relevant change is hours old and
the cost has compounded.

### Solution

A cold-window analysis pane that, on a configurable tick (default 2s,
load-aware), re-reads authoritative state, recomputes metrics from scratch,
ranks hints, raises alerts within a 4-tier severity model, and surfaces
external research findings on a pull-not-push basis. Available in both
TUI (extends `skrills-dashboard`) and browser (extends `skrills-server`)
surfaces, both rendering bit-identical data from a single snapshot.

### Goals

- Continuous awareness of plugin / skill / command / subagent / workflow
  health without polling commands.
- Token-cost itemization at a granularity that lets users act (per skill,
  per plugin, per MCP, per conversation).
- Alert hygiene: alerts that earn their interruption — fewer than 12
  user-visible alerts per hour in steady state.
- Research that arrives when relevant, not when announced — pull-only
  side panel, never injected proactively.
- Equal access from terminal and browser, with no client-side toolchain
  required.

### Non-Goals

- Editing plugins, skills, or MCP configs through the cold-window. This
  is a read-only observability surface; mutation goes through existing
  CLI commands.
- Replacing the existing on-demand `dashboard` invocation. The
  cold-window extends it with continuous mode; one-shot mode remains.
- Multi-user / multi-tenant. The pane is single-user, single-host.
- Mobile / phone surfaces. Browser support targets desktop browsers.

---

## 2. User Scenarios

### 2.1 Persona: Skill author (Aria)

Aria writes and maintains five skrills plugins used by herself and three
teammates. She wants to know, while she works, when one of her changes
has bumped a skill above its token budget or broken a downstream
recommendation.

**Primary flow**:
1. Aria starts the cold-window: `skrills dashboard --watch`
2. She edits a skill file in her IDE.
3. Within one tick (2s), the cold-window's token panel updates and the
   per-skill ledger shows her edit's effect.
4. If she pushed the skill above 10K tokens, a CAUTION-tier alert appears
   in the alert pane (panel-visible, no audible interruption).
5. She acknowledges the alert with a single keystroke.

**Pull-only research**: If the cold-window's research worker has fetched
findings about token-conservation patterns from HN/GitHub, those appear
in the research side panel. Aria opens that panel only when curious;
research never interrupts editing.

### 2.2 Persona: Team lead (Bao)

Bao oversees a fleet of plugins and skills used by his eight-person team.
He needs to spot waste, regressions, and configuration drift across the
ecosystem.

**Primary flow**:
1. Bao opens the browser surface at `http://localhost:8888/dashboard`.
2. The page renders the same data Aria sees in her TUI — token ledger,
   alert pane, hint pane, research panel.
3. He filters the hint pane by severity (`SHOW WARNINGS ONLY`).
4. He sees three skills flagged as "redundant capability" — the hint
   scorer ranked them highest because they affect the most-used skills.
5. He clicks a hint to expand its evidence and reasoning.

**Token budget visibility**: Bao has set the team's budget to 100K
tokens of MCP+skill overhead. The cold-window's status bar shows
"68K / 100K" as a progress indicator; at 80K (80%) a WARNING tier alert
fires; at 100K a hard kill-switch refuses further sync operations.

### 2.3 Persona: AI-tool heavy user (Cy)

Cy uses Claude Code daily and is sensitive to token costs after a
"$15 in 10 minutes" runaway last quarter. He wants confidence that the
system will catch the next blowup before he does.

**Primary flow**:
1. Cy starts skrills with `--alert-budget 50000` (50K tokens of overhead).
2. He works normally; the cold-window runs as a tmux split-pane TUI.
3. A new MCP server he installed adds 35K tokens of tool descriptions.
4. Within one tick, the token panel shows the new MCP's contribution.
5. At 50% (25K) of his budget, the system emits an ADVISORY (panel-only).
6. At 80% (40K) it emits a WARNING (panel-visible + amber-flash).
7. At 100% (50K) it emits a hard CAUTION + locks further sync until
   acknowledged.

**Adaptive cadence**: When Cy is actively typing into Claude (file
mtime in last 10s on his prompt files), the tick rate halves to 1s for
faster feedback. When his machine is under heavy load (compiling the
Rust workspace), the tick rate backs off to 4s or 8s so the cold-window
doesn't compete for cycles.

### 2.4 Persona: External integrator (Dev) [v0.9.0+]

Dev is writing an IDE plugin that surfaces skrills health inside VS Code.
She needs a machine-readable stream of snapshots.

**Future flow** (gRPC roadmap, not v0.8.0):
1. Dev points her IDE plugin at the skrills gRPC endpoint.
2. She subscribes to `WindowService::stream_snapshots()`.
3. Snapshots arrive as binary protobuf on every tick.
4. Her plugin renders them in the IDE sidebar.

This persona is documented to ensure the wire-format crate stays
proto-friendly today. v0.8.0 ships SSE+HTML only.

---

## 3. Functional Requirements

### 3.1 FR1: Tick lifecycle

The cold-window runs a tick on a configurable cadence. Each tick:
1. Re-reads authoritative state (no warm cache).
2. Produces exactly one immutable snapshot.
3. Broadcasts the snapshot to all subscribers (TUI, browser, research
   worker).
4. Completes in under 200ms p99 on a fixture of 200 skills + 50 commands
   - 20 plugins.

The tick must never silently skip. If a tick exceeds its budget, the
next tick is delayed proportionally and a STATUS-tier alert is recorded.

### 3.2 FR2: Single snapshot artifact

Every tick produces one `WindowSnapshot` value. The TUI and browser
surfaces render the same snapshot. Drift is measurable: a parity test
fixture shows both surfaces produce equivalent output for the same
snapshot fields.

### 3.3 FR3: Token ledger

The snapshot includes a token ledger that attributes system-prompt and
session tokens to source:
- Per skill (frontmatter + body, separately).
- Per plugin (sum of skills + commands + agents).
- Per MCP server (tool descriptions especially).
- Per conversation (cache reads vs writes).

Attribution must reach ≥95% accuracy on a known fixture (5 plugins, 30
skills, 3 MCPs). The ledger updates on every tick when source files
change; otherwise widget freshness flags suppress recomputation.

### 3.4 FR4: Four-tier alert model

Every alert carries:
- A **severity tier**: WARNING | CAUTION | ADVISORY | STATUS.
- A **threshold band**: `(low, low_clear, high, high_clear)` for
  hysteresis; `*_clear` values must be re-crossed before re-arming.
- A **min-dwell**: condition holds for K consecutive ticks before firing.
- A **fingerprint**: stable identifier for grouping and dedup.

User-facing behavior:
- WARNING: panel-visible, audible bell (terminal `\a` or browser
  notification), requires per-alert dismissal.
- CAUTION: panel-visible, amber visual flash, dismissible by master-ack.
- ADVISORY: panel-visible only, dismissible by master-ack.
- STATUS: panel-visible only, auto-clears on next tick if condition
  resolves.

### 3.5 FR5: Master-acknowledge

A single keystroke (TUI: `A`; browser: `Alt+A`) clears all CAUTION,
ADVISORY, and STATUS alerts simultaneously. WARNING alerts must be
individually acknowledged. After master-ack, a confirmation toast
shows the count cleared.

### 3.6 FR6: Hint pane

The snapshot includes a ranked list of `Hint` entries produced by the
intelligence crate's recommender. Hints are sorted by score (descending),
filterable by category (token, validation, redundancy, sync-drift,
quality), and pinnable by user (pinned hints stay top).

The default hint scoring formula is documented (§ 6.3) and overridable
via the `HintScorer` trait.

### 3.7 FR7: Research panel (pull-only)

The cold-window includes a research side panel populated asynchronously
by a tome worker. The panel is collapsed by default; users open it
when curious. Research findings never appear in the alert or hint
panes.

The research worker:
- Receives snapshot diffs and identifies novel topic fingerprints.
- Groups, deduplicates, and inhibits redundant fetches via an
  AlertManager-style layer.
- Respects a token-bucket quota at the dispatcher (default: 10 fetches
  per hour, configurable via `--research-rate`).
- Sources via existing `skrills-tome` channels: GitHub, HN/Lobsters,
  papers, TRIZ.

### 3.8 FR8: Browser surface (SSE + HTML fragments)

A browser endpoint at `/dashboard` serves the cold-window. The server:
- Renders an initial HTML page with empty panel slots and an
  `EventSource` to `/dashboard.sse`.
- On every snapshot, emits an SSE event per panel containing pre-rendered
  HTML fragments.
- Uses `axum::Sse::keep_alive()` to stay connected across NATs/proxies.

Browser parity acceptance: a Playwright (or similar) test renders both
surfaces against the same fixture and compares semantic content.

### 3.9 FR9: TUI surface (extension of skrills-dashboard)

The existing `skrills-dashboard` TUI gains:
- A subscriber to the snapshot bus, replacing direct metrics-collector
  reads.
- Three new panels: alert pane (4-tier sorted), hint pane (ranked +
  filterable), research pane (collapsed by default).
- A status bar widget showing current tick cadence and adaptive state
  (e.g., `tick: 2.0s [base]`, `tick: 4.0s [load 0.78]`,
  `tick: 1.0s [active edit]`).

### 3.10 FR10: Configuration surface

CLI flags (passed to `skrills dashboard --watch`):
- `--tick-rate <duration>` (default `2s`)
- `--no-adaptive` (disable load-aware cadence)
- `--alert-budget <tokens>` (default `100000`)
- `--research-rate <per-hour>` (default `10`)
- `--browser` (also start HTTP server on default port)
- `--port <port>` (default `8888`)
- `--no-bell` (suppress audible WARNING bell)

Configuration values appear in the status bar so users see the active
profile. A config file at `~/.skrills/cold-window.toml` may override
defaults; CLI flags override file values.

### 3.11 FR11: Plugin participation

Third-party skrills plugins may opt into the cold-window by shipping
either:
- `<plugin>/health.toml` — declarative checks and thresholds, parsed
  on each tick.
- `<plugin>/health.rs` (compiled into the plugin) — typed `check()`
  function returning a `HealthReport`.

The discovery walker finds these without core changes; the analyzer
includes their reports in the snapshot.

### 3.12 FR12: Hard kill-switch

If cumulative tokens exceed the user-configured `--alert-budget` ceiling:
- A WARNING-tier alert fires with severity `Critical`.
- Subsequent sync operations refuse with a clear error message
  ("token budget exceeded; raise budget or reduce skills").
- The cold-window continues running; only mutating operations are
  blocked.
- The block is cleared by master-ack of the WARNING (with confirmation
  prompt) or by raising the budget.

---

## 4. Success Criteria

### 4.1 Performance

- **SC1**: Median tick duration under 50ms; p99 under 200ms — measured
  on a fixture of 200 skills + 50 commands + 20 plugins.
- **SC2**: Browser surface renders the first paint within 1 second of
  page load on `localhost`.
- **SC3**: TUI startup to first snapshot under 500ms.

### 4.2 Correctness

- **SC4**: TUI and browser render the same snapshot — automated parity
  fixture verifies semantic equivalence of all displayed fields.
- **SC5**: Token attribution accuracy ≥95% on the canonical fixture
  (deviations attributable to tokenizer heuristic, documented).
- **SC6**: Cold rewalk catches a skill mutation within one tick interval
  (manually verified: edit → save → wait one tick → confirm).

### 4.3 Alert hygiene

- **SC7**: Synthetic chaos test (rapid skill mutations over 10 minutes)
  produces fewer than 12 user-visible alerts/hour after hysteresis +
  min-dwell + tier filtering applied.
- **SC8**: Master-ack clears all non-WARNING alerts in a single keystroke;
  WARNING alerts remain.
- **SC9**: Adaptive thresholds adjust to rolling baselines within 5
  minutes of cold-start; pre-warmup falls back to constants without
  spurious alerts.

### 4.4 Resource budget

- **SC10**: Research dispatcher refuses to exceed `--research-rate`
  even under continuous churn (token-bucket invariant preserved).
- **SC11**: Hard kill-switch fires deterministically at 100% of
  `--alert-budget`; cold-window remains responsive after firing.
- **SC12**: Adaptive cadence backs off under load — verified by
  synthetic CPU stressor: tick rate doubles when loadavg/cores > 0.7.

### 4.5 Usability

- **SC13**: Status bar shows current cadence and adaptive state,
  updating each tick.
- **SC14**: Research panel default-collapsed; opening it requires a
  single keystroke / click; new findings increment a badge counter
  without auto-opening.
- **SC15**: Plugin opt-in via `health.toml` adds the plugin's checks to
  the snapshot within one tick of file creation; no core changes
  required.

---

## 5. Edge Cases

- **EC1**: Tick budget exceeded — log a STATUS alert, delay next tick,
  do not silently skip. After three consecutive overruns, emit an
  ADVISORY-tier alert about probable misconfiguration (tick rate too
  aggressive for plugin tree size).
- **EC2**: Discovery walk fails (file system error, permission) — emit
  a WARNING-tier alert with the failure cause; render the previous
  snapshot with a "stale" badge until recovery.
- **EC3**: Tome external API rate-limited — cache the rate-limit response,
  pause the dispatcher for the cooldown period, surface a STATUS alert,
  and resume when the cooldown expires.
- **EC4**: SSE connection dropped — `axum::Sse::keep_alive()` reconnect;
  if the browser misses N snapshots, the next snapshot's HTML fragment
  contains a "reconnecting" badge until the EventSource recovers.
- **EC5**: Plugin `health.toml` malformed — emit a CAUTION-tier alert
  naming the plugin and the parse error; exclude that plugin from the
  snapshot until fixed.
- **EC6**: Snapshot diff identical for many consecutive ticks (idle
  ecosystem) — ticks still fire (correctness floor) but per-widget
  dirty flags suppress recomputation; CPU overhead bounded.
- **EC7**: Hard kill-switch fires while a sync operation is in flight —
  the in-flight operation completes (no partial-write); subsequent
  operations refuse with the kill-switch error.
- **EC8**: Allostatic baseline becomes pathological (e.g., unbounded
  growth) — emit an ADVISORY-tier alert about meta-threshold drift;
  surface "thresholds may need re-rationalization" hint.
- **EC9**: User runs both TUI and browser simultaneously against the
  same skrills daemon — both subscribe to the snapshot bus; render
  identically; no special coordination required.
- **EC10**: User changes `--tick-rate` mid-session via config-file
  reload — applies on next tick; status bar reflects new value.

---

## 6. Contribution Point Contracts

Each remaining contribution point is a trait or fn with a default
implementation. Users override during execute phase if their domain
knowledge prefers different behavior.

### 6.1 `AlertPolicy::evaluate`

```rust
pub trait AlertPolicy: Send + Sync {
    fn evaluate(
        &self,
        prev: &WindowSnapshot,
        curr: &WindowSnapshot,
        history: &AlertHistory,
    ) -> Vec<Alert>;
}
```

**Default**: `LayeredAlertPolicy` — applies hysteresis + min-dwell +
4-tier classification per § 3.4. Threshold values (per § 3.3 token
ledger) come from the user's `--alert-budget` plus the
fixed-percentage defaults (50/80/100%).

**Override use cases**: per-team threshold customization, per-plugin
strictness levels, ML-driven threshold prediction.

**Contract test**: given a fixture sequence of snapshots with known
token trajectories, the default policy produces exactly the expected
alert sequence (golden file).

### 6.3 `HintScorer::rank`

```rust
pub trait HintScorer: Send + Sync {
    fn rank(&self, hints: Vec<Hint>) -> Vec<ScoredHint>;
}
```

**Default**: `MultiSignalScorer` extending the existing
`intelligence::recommend::scorer::RecommendationScorer`. Score formula:

```
score = (frequency * IMPACT_WEIGHT + impact * ACTIONABILITY_WEIGHT)
        / (ease_score + 1.0)
        * recency_factor
        + user_pin_boost
```

Default weights: `IMPACT_WEIGHT = 2.0`, `ACTIONABILITY_WEIGHT = 1.5`.
`recency_factor = exp(-age_days / HALF_LIFE_DAYS)` with
`HALF_LIFE_DAYS = 14`.

**Override use cases**: severity-first ordering for incident response,
ease-first ordering for "low-hanging fruit" mode.

**Contract test**: given a mixed fixture of hints (high-frequency low-
impact vs low-frequency high-impact), the default scorer produces a
deterministic ranking matching the documented formula.

### 6.4 `ResearchBudget::should_query`

```rust
pub trait ResearchBudget: Send + Sync {
    fn should_query(
        &self,
        snapshot: &WindowSnapshot,
        topic_fingerprint: &str,
        last_query: Option<Instant>,
    ) -> bool;
}
```

**Default**: `BucketedBudget` — token-bucket with capacity
`--research-rate`, refill rate `--research-rate / hour`, plus a
per-fingerprint TTL of 1 hour (don't re-query the same topic within
that window unless its underlying fingerprint changes).

**Override use cases**: aggressive research mode for novel projects,
quota-conservative mode for paid API tiers.

**Contract test**: rapid succession of identical fingerprints results
in exactly one query within the TTL; distinct fingerprints respect the
bucket capacity.

### 6.5 `SnapshotDiff::is_alertable`

```rust
pub trait SnapshotDiff: Send + Sync {
    fn is_alertable(&self, prev: &WindowSnapshot, curr: &WindowSnapshot)
        -> Vec<DiffField>;
}
```

**Default**: `FieldwiseDiff` — declarative per-field rules:
- Token counts: alert on ±2% change (kills heuristic noise).
- Skill list: alert on add/remove (any change).
- Validation status: alert on transition (true→false more sensitive
  than false→true).
- Plugin count: alert on add/remove.
- Timestamps: never alert (always change, never meaningful).

**Override use cases**: stricter mode for compliance environments
(any field change alerts), looser mode for high-churn dev environments.

**Contract test**: synthetic snapshots with timestamp-only changes
produce no alerts; snapshots with skill additions produce alerts even
when token counts are within tolerance.

---

## 7. Dependencies

### Internal (existing crates)

| Crate | What we use | New surface added? |
|---|---|---|
| `skrills-discovery` | `walk()` for re-reading plugin tree | No |
| `skrills-analyze` | `tokens::count_tokens` extended for per-source attribution | Yes: `cold_window` module |
| `skrills-intelligence` | `recommend::scorer` extended for hint ranking | Minor: `HintScorer` trait |
| `skrills-metrics` | `MetricsCollector::quantile_over_window` for baselines | Yes: rolling-baseline query |
| `skrills-tome` | Existing research clients via new dispatcher | Yes: `dispatcher` module |
| `skrills-dashboard` | Subscribe to snapshot bus, render new panels | Yes: alert/hint/research panes |
| `skrills-server` | New `/dashboard` + `/dashboard.sse` endpoints | Yes: cold_window API module |
| `skrills-state` | Optional snapshot history persistence | Optional |
| `skrills-snapshot` (NEW) | Wire-format types | Entire crate |

### External (Cargo dependencies)

- `axum` (already in `skrills-server`) — SSE support via
  `axum::response::Sse`.
- `tokio::sync::broadcast` (already used) — snapshot bus.
- `time` (already in workspace) — timestamps.
- `serde` + `serde_json` (already in workspace) — wire format.
- `num_cpus` (NEW, ~5 KB) — `num_cpus::get()` for adaptive cadence.

No new heavy dependencies. No JS framework. No protobuf compiler in
v0.8.0 (deferred to v0.9.0 with the gRPC follow-up).

---

## 8. Assumptions

- **A1**: Token estimation remains character-heuristic (`analyze::tokens`
  ratios). We do not introduce a real tokenizer in v0.8.0; itemization
  inherits the existing imprecision but is consistent.
- **A2**: Linux-style `loadavg` is available via `/proc/loadavg` or
  equivalent. macOS uses `sysctl`; Windows uses a fallback (per-core CPU
  utilization). The adaptive cadence policy degrades gracefully if
  loadavg is unavailable (reverts to fixed cadence).
- **A3**: Users running the browser surface have a modern desktop
  browser with EventSource support (Chrome, Firefox, Safari ≥10,
  Edge). IE not supported.
- **A4**: SSH-friendly TUI: no escape codes leaking from the analyzer.
  Tested via `ssh user@host -t skrills dashboard --watch` smoke test.
- **A5**: Plugin authors who opt into `health.toml` accept that their
  declared thresholds become part of the alert budget; the cold-window
  does not silently sandbox third-party reports.
- **A6**: The user runs at most one skrills daemon per host. No
  cross-instance coordination.

---

## 9. Technical Constraints

(From `docs/cold-window-brief.md` § 4 and § 5.)

- Wire-format crate `skrills-snapshot` is the single source of truth.
  TUI and browser depend on it; not on each other.
- Browser transport is SSE + HTML fragments via `axum::Sse`. No JS
  framework. No client-side state.
- TUI extends `skrills-dashboard` via subscription to the snapshot bus.
- Adaptive cadence uses load-aware policy with 0.7 / 0.9 thresholds.
- Wire-format types are designed proto-friendly for v0.9.0 gRPC.
- All code paths remain `unsafe`-free (existing
  `#![deny(unsafe_code)]`).

---

## 10. Out of Scope

- gRPC service surface (deferred to v0.9.0 follow-up).
- Mutating operations through the cold-window UI.
- Multi-user / multi-tenant.
- Mobile / phone surfaces.
- Real (not heuristic) tokenizer integration.
- Cross-host coordination of daemons.
- Custom theming / color schemes (use existing dashboard defaults).

---

## 11. Open Items for Plan Phase

- [ ] Concrete TDD test fixtures for SC1–SC15 (which already exist as
  workspace fixtures vs which need creation).
- [ ] Phase ordering for crate touches: bottom-up (snapshot → analyze
  → metrics → intelligence → dashboard → server) seems correct.
- [ ] Risk classification per task per `leyline:risk-classification`
  (most are GREEN; the `cold_window::engine` work is YELLOW).
- [ ] War-room gate triggered by RED tasks if any emerge — none expected
  given the existing seam, but the plan-review module flags it.

---
