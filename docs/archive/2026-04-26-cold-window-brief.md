# Cold-Window Real-Time Analysis: Project Brief

**Status**: Draft — design decisions resolved; ready for spec phase
**Date**: 2026-04-26
**Branch**: `cold-window-analysis-0.8.0`
**Author**: skrills team
**Phase**: brainstorm (attune mission Phase 1)

---

## 1. Mission Charter

### Outcome

Add a **cold-window real-time analysis** subsystem to skrills that surfaces the
health of a user's plugin/skill/command/subagent/workflow ecosystem in a
continuously refreshing pane. The pane runs in two surfaces — a TUI (extending
the existing `skrills-dashboard` crate) and a browser (served by the existing
`skrills-server` crate) — both rendering the same authoritative snapshot.

The pane refreshes on a configurable tick (default 2s), recomputes from
authoritative state on every tick, raises alerts when thresholds cross, surfaces
hints / suggestions / meta-improvements, and pulls research findings from
external channels (HN, GitHub, papers via the existing `skrills-tome` crate)
asynchronously and within a budgeted quota.

### Why "cold window"

Each tick re-reads authoritative state from disk and recomputes — no warm-cache
shortcuts. This trades CPU for correctness: drift between observed and actual
state becomes structurally impossible. The Anthropic and community discourse
(Anthropic context-engineering guide, Willison "context rot", AugmentCode "junk
drawer") has converged on cold-refresh as the correct default for monitoring
surfaces past a complexity threshold; warm-state is treated as a liability.

### Success metric

- TUI and browser surfaces render bit-identical data, every tick, with
  measured render-skew under one tick interval.
- Alerts respect a 4-tier hierarchy with hysteresis + min-dwell, producing
  fewer than 12 user-visible alerts per hour in steady-state operation.
- Token-cost itemization correctly attributes >95% of system-prompt tokens to
  source plugins / skills / MCPs (validated against a known fixture).
- External research fetches respect a token-bucket quota; the dispatcher never
  exceeds the user-configured rate even under continuous churn.

### Constraints

- No new top-level dependencies if existing crate dependencies suffice.
- Render path must remain `unsafe`-free (existing `#![deny(unsafe_code)]`).
- Browser surface must work without a JS framework (htmx-style fragment swap).
- TUI surface must stay SSH-friendly (no terminal escape codes leaking from
  the analyzer layer).
- Token research budget bounded by `tome` crate's existing cache + rate limits.

### Stop criteria

- Architectural fork resolved (transport choice + tick rate)
- 7 research-backed deltas (§ 5) reviewed and either accepted or amended
- Five user-decision contribution points (§ 6) scoped
- Spec phase greenlit by user

---

## 2. Existing Seam (What We're Mounting Onto)

The skrills workspace already has the crates we need. The cold-window is an
**extension of the existing seam**, not greenfield:

| Crate | Existing role | Cold-window role |
|---|---|---|
| `skrills-discovery` | Walk plugin tree, surface skills/commands/agents | Authoritative source of truth re-walked each tick |
| `skrills-analyze` | Token estimation, dependency graph, optimization scoring | Token ledger + per-source attribution |
| `skrills-intelligence` | Smart recommendations (similarity, usage, context) | Hint generator with score breakdown |
| `skrills-metrics` | SQLite metrics collector + broadcast channel | Historical baseline for adaptive thresholds |
| `skrills-tome` | Multi-channel research (GitHub, HN, papers, TRIZ) | Async research worker behind quota gate |
| `skrills-dashboard` | Ratatui+crossterm TUI with `Tick` event | TUI surface, subscribes to snapshot bus |
| `skrills-server` | Axum HTTP transport (rmcp + streamable-http) | Browser surface via SSE + HTML fragments |
| `skrills-state` | State persistence | Optional snapshot history |

**Key observation**: `dashboard::events::EventHandler` already drives a
`tokio::time::interval(tick_rate)` and `metrics::MetricsCollector` already
exposes a `tokio::sync::broadcast::Sender<MetricEvent>`. The cold-window
mounts onto these primitives — we don't add a new scheduler.

---

## 3. Architecture

```
                       ┌──────────────────────────────────┐
                       │  ColdWindowEngine                 │
                       │  (new module in skrills-analyze)  │
                       │  ──────────────────────────────   │
                       │  fn tick(t: Instant)              │
                       │   -> Arc<WindowSnapshot>          │
                       │                                   │
                       │  re-walks discovery ──┐           │
                       │  recounts tokens ─────┤           │
                       │  computes hints ──────┼─► snapshot│
                       │  diffs vs prev ───────┤           │
                       │  emits alerts ────────┘           │
                       └────────────┬──────────────────────┘
                                    │
                       ┌────────────┴───────────┐
                       │   SnapshotBus           │
                       │   broadcast<Arc<Snap>>  │   ← single source of truth
                       └────────────┬───────────┘
        ┌────────────────────────┬──┴───────────────────────┐
        ▼                        ▼                          ▼
┌─────────────────┐    ┌──────────────────┐     ┌───────────────────┐
│ TUI Adapter     │    │ HTTP Adapter      │     │ Tome Worker       │
│ ──────────      │    │ ─────────         │     │ ─────────         │
│ skrills-        │    │ skrills-server    │     │ async, quota-     │
│   dashboard     │    │ /dashboard.sse    │     │ gated; attaches   │
│ ratatui render  │    │ HTML fragments    │     │ findings to next  │
│                 │    │ via axum::Sse     │     │ snapshot          │
└─────────────────┘    └──────────────────┘     └───────────────────┘
```

### Wire-format crate split (research-backed)

Following the `tokio-console` pattern (`console-api` / `console-subscriber` /
`tokio-console`), we add a small **wire-format crate**:

```
skrills-snapshot   ← new crate; all shared types + serde
   │
   ├── WindowSnapshot
   ├── Alert (4-tier severity)
   ├── Hint (scored, ranked)
   ├── ResearchFinding
   └── TokenLedger
```

`dashboard` and `server` depend on `skrills-snapshot` and on each other
**not at all**. The producer (`analyze::cold_window`) emits this struct;
both surfaces consume it. Drift between TUI and browser is structurally
impossible because the artifact is the contract.

### Data flow per tick

1. Tick fires (default 2s, configurable via CLI flag like `rustnet -r`).
2. `ColdWindowEngine::tick()`:
   - Re-walk discovery (`skrills-discovery::walk`)
   - Recompute token ledger per source (extends `analyze::tokens`)
   - Pull last metrics from `MetricsCollector` (rolling baseline)
   - Run hint generator (extends `intelligence::recommend::scorer`)
   - Diff against previous snapshot (hash-based, semantic-aware via
     `SnapshotDiff::is_alertable`)
   - Run alert evaluator (4-tier × hysteresis × min-dwell)
   - Attach any newly-arrived `ResearchFinding`s from the tome worker
3. Wrap in `Arc<WindowSnapshot>`, send on `SnapshotBus`.
4. Subscribers render:
   - TUI: ratatui reads the new `Arc`, swaps panel state, redraws
   - HTTP: axum SSE handler renders HTML fragments per panel,
     pushes via `text/event-stream`
   - Tome worker: inspects diff for novelty fingerprints; if novel and
     within quota, dispatches research query

### Per-widget freshness floor (bottom-pattern)

Not every widget recomputes every tick. Each widget exposes:

```rust
trait Widget {
    fn freshness(&self) -> Duration;   // max age before forced refresh
    fn dirty(&self, prev: &Snap, curr: &Snap) -> bool;  // skip if !dirty
}
```

A global `STALE_FLOOR_MS` (default 30s, mirroring `bottom`'s value)
caps how stale any widget's data may be regardless of `dirty`.

---

## 4. Architectural Forks (Resolved)

### 4.1 Browser transport — SSE + HTML, gRPC roadmapped

**Decision**: SSE + HTML fragments for the browser surface; in-process
`tokio::sync::broadcast` channel between analyzer and TUI (no RPC inside
the binary). gRPC is roadmapped as a follow-up external-client surface
(§ 5.8), not shipped in v0.8.0.

**gRPC was considered seriously.** tokio-console — whose 3-crate split
we already borrowed for the `skrills-snapshot` wire-format pattern —
ships its data over gRPC via `console-api` + `tonic`. Two factors push
it to a follow-up release for skrills:

1. **Browsers cannot speak native gRPC.** HTTP/2 trailers aren't exposed
   to JavaScript, so a browser surface needs gRPC-Web via `tonic-web`
   or Envoy (proxy hop) plus a generated JS/TS client (~100–300 KB
   bundle). Both add toolchain complexity and break the "open localhost,
   no setup" UX that SSE delivers natively.
2. **The TUI runs in-process.** Analyzer and TUI live in the same binary;
   an RPC layer between them is pure overhead for v1.

**Net for v0.8.0**: SSE for the network surface, in-process channel for
TUI, single `skrills-snapshot` wire-format crate as the contract.
The wire-format types are designed proto-friendly (§ 5.8) so a `tonic`
gRPC service can be added in v0.9.0 without disturbing the browser or
TUI. External consumers (IDE plugins, machine scrapers, MCPs wanting
binary streams) become the gRPC value prop when we ship that surface.

### 4.2 Tick cadence — 2s base, load-aware adaptive

**Decision**: 2s base tick rate, configurable via `--tick-rate <duration>`
flag. Adaptive policy enabled by default (`--no-adaptive` to disable).
The effective cadence appears in a status-bar widget so users see when
the system has slowed or sped up.

**Adaptive policy** (concrete):

```text
on tick T, sample LoadSample { loadavg_1min: f64, last_edit: Option<Instant> }

  recent edit (<10s)?
    yes → tick = max(base / 2, min)               // user is active, freshen
    no  → load_ratio = loadavg_1min / num_cpus
            ratio > 0.9  → tick = min(base * 4, max)   // heavily loaded
            ratio > 0.7  → tick = min(base * 2, max)   // moderately loaded
            else         → tick = base
```

Defaults: `base = 2s`, `min = 500ms`, `max = 8s`, `cores = num_cpus::get()`.
All overridable via CLI flags. Backoff multipliers are powers of two so
they compose with hysteresis without oscillating at the boundary.

**Why 0.7 and 0.9**: same load-ratio thresholds the Linux scheduler uses
for "moderately loaded" vs "heavily loaded" classifications. Borrowed
intuition, no new tuning constants invented.

---

## 5. Research-Backed Design Deltas

These are amendments to the original sketch driven by the three parallel
tome research tracks (GitHub code search, HN/Lobsters discourse, TRIZ
cross-domain analogies).

### 5.1 Wire-format crate split

**Source**: `tokio-console` (3-crate split), Bloomberg Terminal architecture
(TRIZ bridge: data-center normalization → snapshot artifact),
`PerkyZZ999/ZManager` (4-crate shared-core pattern).

Add `skrills-snapshot` as the contract crate. Producers depend on it; both
surfaces depend on it. No surface depends on the other.

### 5.2 Two-axis alert model

**Source**: FAA AC 25.1322-1 cockpit CAS (4-tier WARNING/CAUTION/ADVISORY/STATUS),
ISA-18.2 alarm management (hysteresis + min-dwell), CHI 2025 proactive-AI
study (pull-not-push).

Each alert has:
- **Severity tier**: WARNING (red, push notification + audible) | CAUTION
  (amber, panel-visible + visual cue) | ADVISORY (cyan, panel-only) |
  STATUS (white, panel-only, dismissible)
- **Threshold band**: `(low, low_clear, high, high_clear)` — must cross
  back through `*_clear` before re-arming. Kills chatter.
- **Min-dwell**: condition holds for K consecutive ticks before firing.
  Eliminates fleeting alarms.
- **Master-acknowledge**: single keystroke clears all CAUTION + ADVISORY
  - STATUS at once; only WARNING requires per-alert dismissal.
- **Priority sequence**: deterministic ordering when multiple alerts fire
  same tick (no race-condition display order).

### 5.3 Itemized token ledger

**Source**: HN "Expensively Quadratic" thread (Feb 2026), Willison "too
many MCPs" (Aug 2025), DEV.to AI cost-blowup war stories.

Token ledger separates:
- Per-skill tokens (frontmatter + body)
- Per-plugin total
- Per-MCP server tokens (tool descriptions are the largest invisible drain)
- Conversation tokens (cache reads dominate past 20K)

**Alert thresholds**:
- 20K cumulative tokens → CAUTION (quadratic inflection)
- 50K → ADVISORY (Willison MCP-overhead range)
- 80% of user-configured budget → WARNING
- 100% of budget → hard kill-switch (real war stories show soft warnings
  alone fail)

### 5.4 AlertManager-style research dispatcher

**Source**: Prometheus AlertManager (TRIZ bridge: monitoring → research-pull),
CHI 2025 proactive-AI study.

Insert a layer between "interesting change detected" and "fetch external
research":
- **Fingerprint** by topic key (skill name + change kind)
- **Group** within a TTL window — flapping skill yields one fetch, not 100
- **Inhibit** — if a higher-confidence finding already covers the topic
  in tome's cache, skip the redundant fetch
- **Token-bucket quota** at the dispatcher — never exceed configured rate
  even under continuous churn

Research findings are **pull-only** in the UI: a side panel surfaces
available findings; nothing interrupts the user. CHI 2025's study found
that proactive injection is rated negatively ("distracting", "annoying").

### 5.5 Allostatic adaptive thresholds

**Source**: TRIZ bridge from physiology (allostasis vs homeostasis),
ISA-18.2 alarm-rate budget.

Thresholds are **functions of context**, not constants:

```
threshold = baseline_quantile(window_history) + k * std_dev
```

Maintain a rolling baseline (EWMA or quantile) per metric. Alert on
deviation from expected-given-context, not deviation from a hardcoded
number. Track **alerts-per-hour** as a meta-signal — if sustained >12/hr,
the dashboard surfaces "thresholds may need re-rationalization" before
surfacing more alerts. The system rates its own threshold quality.

### 5.6 Per-widget freshness with global stale-floor

**Source**: ClementTsang/bottom (`STALE_MIN_MILLISECONDS`,
`force_update_data` per widget).

Each widget decides whether the global tick applies via a `dirty()`
predicate. Token panel only re-reads when session log mtime changes.
Validation panel only reruns when a skill file changes. A
`STALE_FLOOR_MS = 30_000` ensures no widget displays data older than 30s
regardless of dirtiness.

### 5.7 Convention-based plugin participation

**Source**: neovim health framework (`<plugin>/health.lua` with `check()`).

Any third-party skrills plugin can opt into the cold-window analysis by
shipping a `health.toml` (or `health.rs` for compiled checks) that exposes
a typed `check() -> HealthReport`. Discovery walks find these and the
analyzer aggregates them into the snapshot. New plugins participate
without core changes.

### 5.8 Proto-friendly wire-format with gRPC roadmapped

**Source**: tokio-console pattern (gRPC over wire-format crate), weighed
against browser-first UX (§ 4.1).

`skrills-snapshot` types are designed proto-translatable from day one,
even though v0.8.0 ships them as JSON over SSE:

- Every enum variant is a tagged union (proto3 `oneof`-compatible) — no
  `#[serde(untagged)]` shortcuts.
- Every collection is `Vec<T>` or `HashMap<K, V>` with primitive keys
  (proto3 `map`-compatible).
- Every `Option<T>` field stays explicit (proto3 wrapper-message-compatible
  on the gRPC side).
- Timestamps use `time::OffsetDateTime` with millisecond precision —
  trivially mappable to `google.protobuf.Timestamp`.

Cost today: zero. The crate ships with `serde` derives, no `prost`
dependency. Cost of the v0.9.0 gRPC follow-up:

1. Add a `proto/skrills.proto` describing the same types.
2. Wire `tonic-build` into the `skrills-snapshot` build script.
3. Implement a `WindowService` that streams snapshots via tonic.
4. Optional: add `tonic-web` middleware behind a `grpc-web` feature flag
   for browser clients that prefer binary streaming.

External consumers get binary streaming without disturbing v0.8.0's
browser/TUI surfaces.

---

## 6. User Decision Contribution Points (Learning Mode)

Five places where domain knowledge shapes behavior. These are the 5–10 line
contributions to capture during the spec phase. Each will be scaffolded with
context, signature, and a clear TODO marker.

### 6.1 `AlertPolicy::evaluate(prev, curr) -> Vec<Alert>`

**Context**: Given two consecutive snapshots, decide which thresholds fired,
which need to debounce, and what tier each alert belongs to. Hysteresis +
min-dwell + 4-tier severity all converge here.

**Trade-offs**:
- Strict thresholds (fewer false alarms) vs sensitive thresholds (catch
  regressions early)
- Per-skill thresholds (precise) vs global thresholds (simple)
- User-pinned WARNING tier overrides

### 6.2 `CadenceStrategy` trait (default-spec'd; extension point)

**Status**: The default `LoadAwareCadence` implementation is now spec'd
in § 4.2 and ships as the v0.8.0 default. This becomes a documented
**extension point** rather than a required user contribution: anyone
who wants different behavior (fixed cadence for load testing, ML-driven
policy, time-of-day modulation) implements the `CadenceStrategy` trait
and passes it to `ColdWindowEngine::with_cadence`.

```rust
pub trait CadenceStrategy: Send + Sync {
    fn next_tick(&self, sample: LoadSample) -> Duration;
}
```

No required contribution; included here so the extension surface is
visible.

### 6.3 `HintScorer::rank(hints) -> Vec<ScoredHint>`

**Context**: Given N hints generated by `intelligence::recommend`, which
go on top? Combines frequency × impact / ease, recency bias, user-pinned
boosts.

**Trade-offs**:
- Frequency-weighted (popular issues bubble) vs impact-weighted (severe
  issues bubble) vs ease-weighted (low-hanging fruit first)

### 6.4 `ResearchBudget::should_query(snapshot, last_query) -> bool`

**Context**: Debounce policy that protects tome's external API quota.
Decides when the cold-window actually calls out to GitHub/HN/papers.

**Trade-offs**:
- Strict TTL (predictable cost) vs novelty-driven (cost spikes on churn)
- Topic-key fingerprinting granularity (per-skill vs per-category)

### 6.5 `SnapshotDiff::is_alertable(prev, curr) -> bool`

**Context**: Hash-based diff is too noisy (timestamps, ordering); pure
semantic diff is too expensive. Decide what counts as "changed enough"
to break dedup.

**Trade-offs**:
- Field allowlist (changes here always alert) vs blocklist (these never
  alert)
- Per-field tolerances (token count ±2% noise)

---

## 7. Crate Map

Concrete files we'll touch in the execute phase:

| Crate | New / modified | What |
|---|---|---|
| `skrills-snapshot` (NEW) | `lib.rs`, `types.rs`, `serde.rs` | Wire-format types: WindowSnapshot, Alert, Hint, ResearchFinding, TokenLedger |
| `skrills-analyze` | NEW: `cold_window/mod.rs`, `cold_window/engine.rs`, `cold_window/diff.rs`, `cold_window/alert.rs`, `cold_window/cadence.rs` | ColdWindowEngine, AlertPolicy, SnapshotDiff, RefreshCadence |
| `skrills-analyze` | EXTEND: `tokens.rs` | Per-source token attribution (skill, plugin, MCP, conversation) |
| `skrills-intelligence` | EXTEND: `recommend/scorer.rs` | HintScorer with rank() + tier mapping |
| `skrills-metrics` | EXTEND: `collector.rs` | Rolling baseline query: `quantile_over_window(metric, duration)` |
| `skrills-tome` | NEW: `dispatcher.rs` | AlertManager-style fingerprint + group + inhibit + token-bucket |
| `skrills-dashboard` | EXTEND: `app.rs`, `ui.rs` | Subscribe to SnapshotBus; alert pane (4-tier); hint pane; research pane |
| `skrills-dashboard` | EXTEND: `events.rs` | Master-acknowledge keybinding |
| `skrills-server` | NEW: `api/cold_window.rs` | `/dashboard.sse` endpoint emitting HTML fragments |
| `skrills-server` | NEW: `ui/cold_window.html`, `ui/fragments/*.html` | HTML templates |
| `skrills-cli` | EXTEND: existing dashboard subcommand | `--tick-rate`, `--alert-budget`, `--browser` flags |

---

## 8. Acceptance Criteria

1. **Parity**: TUI and browser render the same `WindowSnapshot` every tick;
   automated test fixture verifies bit-identical data fields.
2. **Tick budget**: median tick duration <50ms, p99 <200ms on a fixture of
   200 skills + 50 commands + 20 plugins.
3. **Alert budget**: synthetic chaos test (rapid skill mutations) produces
   <12 user-visible alerts/hour after hysteresis + min-dwell + tier
   filtering applied.
4. **Token attribution**: known fixture (5 plugins, 30 skills, 3 MCPs) has
   ≥95% of system-prompt tokens correctly attributed to source.
5. **Research quota**: token-bucket refuses to dispatch when budget exhausted;
   refusals are visible in the dashboard's self-meta panel.
6. **Cold correctness**: deliberately stale-cache test fails; fresh re-walk
   passes. No warm-state sneaks into the snapshot.
7. **Plugin participation**: third-party plugin with `health.toml` appears
   in snapshot without core changes.
8. **Master-acknowledge**: single keystroke clears all CAUTION/ADVISORY/
   STATUS while preserving WARNING tier.

---

## 9. Risk Notes

- **R1**: 2s tick may be too aggressive for users with very large plugin
  trees (>500 skills). Mitigation: adaptive cadence (decision #6.2) +
  per-widget dirty flags (delta 5.6).
- **R2**: SSE keepalive across NATs / proxies has known quirks. Mitigation:
  `axum::Sse::keep_alive()` with sane interval; document as a constraint.
- **R3**: Token estimation is character-heuristic, not real tokenizer.
  Already a known limitation in `analyze::tokens`; itemized ledger inherits
  the imprecision but is consistent.
- **R4**: Allostatic baselines need warmup history; cold-start before warmup
  must fall back to constants. Mitigation: a `WarmupPolicy` with explicit
  fallback rules (decision #6.1 sub-choice).
- **R5**: Tome quota interacts with user-set rate limits and underlying
  source rate limits (HN, GitHub). Mitigation: existing `tome::cache` +
  new dispatcher's token bucket as belt-and-suspenders.

---

## 10. Resolved Decisions

- [x] **§ 4.1**: Transport — SSE+HTML for v0.8.0; gRPC roadmapped for
  v0.9.0 via § 5.8 proto-friendly wire-format
- [x] **§ 4.2**: Tick cadence — 2s base, load-aware adaptive (§ 4.2 policy)
- [x] **§ 5**: All seven original deltas accepted; § 5.8 added
  (gRPC roadmap)
- [x] **§ 6**: Four user contribution points active (§ 6.1, 6.3, 6.4, 6.5);
  § 6.2 spec'd by default with `CadenceStrategy` trait as extension
- [ ] **§ 8**: Acceptance criteria scoped per current draft;
  refine during specify phase if needed

---

## 11. Sources

### Code research (GitHub)
- [tokio-rs/axum SSE example](https://github.com/tokio-rs/axum/blob/main/examples/sse/src/main.rs)
- [tokio-rs/console](https://github.com/tokio-rs/console)
- [ClementTsang/bottom](https://github.com/ClementTsang/bottom)
- [bnjbvr/cargo-machete](https://github.com/bnjbvr/cargo-machete)
- [neovim health.lua](https://github.com/neovim/neovim/blob/master/runtime/lua/vim/health.lua)
- [davidpdrsn/axum-live-view](https://github.com/davidpdrsn/axum-live-view)
- [PerkyZZ999/ZManager](https://github.com/PerkyZZ999/ZManager)
- [terraform-docs/terraform-docs](https://github.com/terraform-docs/terraform-docs)

### Discourse research
- [HN: Expensively Quadratic LLM cost curve](https://news.ycombinator.com/item?id=47000034)
- [Simon Willison: too many MCPs](https://simonwillison.net/2025/Aug/22/too-many-mcps/)
- [Simon Willison: context rot](https://simonwillison.net/2025/Jun/18/context-rot/)
- [Anthropic: Effective context engineering](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents)
- [CHI 2025: Proactive AI Assistants for Programming](https://dl.acm.org/doi/10.1145/3706598.3714002)
- [Augment: Your agent's context is a junk drawer](https://www.augmentcode.com/blog/your-agents-context-is-a-junk-drawer)
- [Yohei Nakajima: Self-Improving AI Agents](https://yoheinakajima.com/better-ways-to-build-self-improving-ai-agents/)

### TRIZ analogies
- [ISA-18.2 Alarm Management Standard (PAS)](https://www.isa.org/getmedia/55b4210e-6cb2-4de4-89f8-2b5b6b46d954/PAS-Understanding-ISA-18-2.pdf)
- [FAA AC 25.1322-1 Flight Deck Alerting](https://www.faa.gov/documentLibrary/media/Advisory_Circular/AC_25.1322-1.pdf)
- [Prometheus AlertManager](https://prometheus.io/docs/alerting/latest/alertmanager/)
- [Bloomberg Terminal Architecture](https://en.wikipedia.org/wiki/Bloomberg_Terminal)
- [Allostasis (PMC)](https://pmc.ncbi.nlm.nih.gov/articles/PMC4166604/)
