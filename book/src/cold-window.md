# Cold-Window Real-Time Analysis

The cold-window is a continuously-refreshing analysis surface for
your skrills ecosystem. It re-reads authoritative state from disk on
every tick (no warm cache shortcuts), runs a four-tier alert policy
with hysteresis and min-dwell over the snapshot, ranks hints with a
recency-weighted scorer, and surfaces external research findings on
a pull-only basis.

Two render targets, both consuming the same `WindowSnapshot`
artifact:

- **TUI**: ratatui-based panes in `skrills-dashboard::cold_window`
  (alert pane, hint pane, research pane, status bar). Available as
  library code today; CLI mounting lands in a follow-up.
- **Browser**: HTML page + Server-Sent Events stream in
  `skrills-server::api::cold_window`. Available as a CLI subcommand
  in v0.8.0.

## Quick start

Run the browser surface against the engine's demo producer:

```bash
skrills cold-window --browser --port 8888
```

Open `http://localhost:8888/dashboard` in any modern browser. The
page renders four panes:

- **Status bar**: tick cadence with adaptive label
  (`tick: 2.0s [base]`, `tick: 4.0s [load 0.78]`,
  `tick: 1.0s [active edit]`), token-budget progress with a colored
  bar (green → cyan → yellow → red), per-tier alert counts, optional
  research-quota remaining.
- **Alerts**: 4-tier list (Warning / Caution / Advisory / Status)
  sorted tier-then-recency. Per-tier coloring; alerts carry a
  hysteresis band so re-arming requires re-crossing the matching
  `*_clear` value.
- **Hints**: ranked by `MultiSignalScorer` formula
  `(frequency × IMPACT_WEIGHT + impact × ACTIONABILITY_WEIGHT)
  / (ease + 1) × exp(-age_days / HALF_LIFE_DAYS)`. Pinned hints sort
  to the top regardless of score.
- **Research**: pull-only side panel. Findings from GitHub, Hacker
  News, Lobsters, papers, and TRIZ analogies arrive asynchronously
  through the tome dispatcher. Empty by default; the dispatcher
  respects a token-bucket quota.

`Ctrl-C` exits cleanly within the 2-second shutdown budget. The
browser sees a `status` event with `reconnecting…` while the server
drains.

## CLI flags

| Flag | Default | Effect |
|---|---|---|
| `--alert-budget <N>` | `100000` | Token-budget ceiling. At 80% a `Warning` alert fires; at 100% the kill-switch engages. |
| `--research-rate <N>` | `10` | Tome dispatcher fetches per hour. The bucket persists across restarts at `~/.skrills/research-quota.json` and refills pro-rata by elapsed time. |
| `--port <N>` | `8888` | Browser HTTP port (only with `--browser`). |
| `--browser` | off | Run the HTTP browser surface. |
| `--no-adaptive` | off | Disable load-aware cadence; fix tick rate to base. |
| `--tick-rate-ms <N>` | `2000` | Override base tick rate. |
| `--skill-dir <DIR>` | (none) | Repeatable. Adds skill directories beyond the defaults. |
| `--plugins-dir <DIR>` | `./plugins` | Plugins root whose `<plugin>/health.toml` files participate in each tick. Missing or unreadable directories yield an empty plugin set without error. |

## Architecture

A single producer (`ColdWindowEngine` in `skrills-analyze::cold_window`)
emits one `Arc<WindowSnapshot>` per tick on a bounded
`tokio::sync::broadcast` channel. Both render targets subscribe to
the same bus. Drift between them is structurally impossible because
the artifact is the contract.

```text
┌────────────────────────────────────────────────────────┐
│  ColdWindowEngine (skrills-analyze::cold_window)       │
│   tick(input) → Arc<WindowSnapshot>                    │
│                                                        │
│   ↳ FieldwiseDiff       (snapshot diff)                │
│   ↳ LayeredAlertPolicy  (4-tier + hysteresis)          │
│   ↳ DefaultHintScorer   (intelligence::MultiSignal)    │
│   ↳ LoadAwareCadence    (load-ratio backoff)           │
└────────────────────┬───────────────────────────────────┘
                     │
        ┌────────────▼─────────────┐
        │  SnapshotBus              │
        │  broadcast<Arc<Snap>>     │
        └────────────┬─────────────┘
   ┌─────────────────┼─────────────────┐
   ▼                 ▼                 ▼
TUI panes        SSE handler     Tome worker
(dashboard)      (server)        (quota-gated)
```

Resource bounds (R11 mitigation): the broadcast channel caps at 16
queued snapshots; lagging subscribers drop and the SSE handler emits
a `status` banner ("subscriber lagged by N ticks") rather than
blocking the producer. The activity ring caps at 100 entries with
oldest-evict.

## Token thresholds

Defaults are research-backed:

- **20K total tokens** → `Advisory` (Anthropic API quadratic-cost
  inflection per the Feb 2026 HN
  [Expensively Quadratic](https://news.ycombinator.com/item?id=47000034)
  analysis).
- **50K total tokens** → `Caution` (Willison's
  [Too many Model Context Protocol servers](https://simonwillison.net/2025/Aug/22/too-many-mcps/)
  range).
- **80% of `--alert-budget`** → `Warning`.
- **100% of `--alert-budget`** → `Warning` + kill-switch engaged
  (mutating sync operations refuse until master-acked).

All thresholds are configurable via builder methods on
`LayeredAlertPolicy` if you embed the engine directly.

## Browser security posture

Two layers of XSS defense:

1. The server `html_escape`s every user-derived string before it
   lands in a fragment.
2. The browser swap path uses `DOMParser` + `replaceChildren`, which
   parses `<script>` tags into nodes that **do not execute** when
   later attached to the document — even if Layer 1 ever regresses,
   an injected payload can't run.

When TLS is configured (`axum-server` + rustls), ALPN advertises
`h2`. Multiple browser tabs in the same origin all stay subscribed
past HTTP/1.1's 6-connection-per-origin limit because HTTP/2
multiplexes streams.

## Plugin participation (FR11)

Third-party skrills plugins opt into the cold-window by shipping a
`health.toml` file alongside their `.claude-plugin/plugin.json`.
Each tick the engine cold-walks the configured plugins root
(`--plugins-dir`, default `./plugins`) and parses every
`<plugin>/health.toml` it finds. Schema:

```toml
plugin_name = "my-plugin"   # optional; defaults to directory name
overall = "ok"              # ok | warn | error | unknown

[[checks]]
name = "smoke"
status = "ok"
message = "all systems nominal"  # optional

[[checks]]
name = "deps"
status = "warn"
```

Plugins without a `health.toml` are silently excluded — a missing
file is the opt-out signal, not an error. **Malformed**
`health.toml` files (parse error, unknown status string) trigger a
deterministic `Caution`-tier alert with a stable fingerprint
(`plugin-health-malformed::<plugin>`) and exclude the plugin from
the snapshot until the file is fixed (spec EC5). Hysteresis and
min-dwell are skipped for these alerts because user configuration
errors need immediate visibility.

## Prior-art validation

The cold-window's design draws explicitly from mature reference
implementations. See `docs/research/tome-dashboards.md` and
`docs/research/tome-alerts-hints.md` for citations and verdicts.

| Pattern | Reference | Skrills' choice |
|---|---|---|
| Single-snapshot fan-out to TUI + browser | [ccboard](https://github.com/FlorianBruniaux/ccboard), [vector top](https://github.com/vectordotdev/vector/pull/4702), [Glances](https://github.com/nicolargo/glances) | `Arc<WindowSnapshot>` over a bounded broadcast channel; both surfaces are pure renderers. |
| Cold rewalk every tick | [Prometheus file_sd](https://github.com/prometheus/prometheus/blob/main/docs/http_sd.md), [fluent-bit `in_tail`](https://github.com/fluent/fluent-bit) | Full filesystem walk per tick within the SC1 200 ms p99 budget; no warm cache. |
| Tick rate vs frame rate separation | [ratatui async-template](https://github.com/ratatui/async-template) | Adaptive cadence (state advance) is decoupled from SSE keep-alive (redraw). |
| Hysteresis + min-dwell + tier filtering | [Prometheus Alertmanager `aggrGroup`](https://github.com/prometheus/alertmanager/blob/main/dispatch/dispatch.go), [ISA-18.2 alarm management](https://github.com/alerta/alerta) | 4-tier model with hysteresis clear ratio 0.95 and min-dwell 2 ticks. |
| Token-bucket quota with restart-resilient persistence | [governor](https://github.com/boinkor-net/governor), Sensu `dedup-key-template` | AlertManager-style research dispatcher with quota persisted at `~/.skrills/research-quota.json`. |
| Defense-in-depth XSS posture | [axum-htmx](https://github.com/robertwayne/axum-htmx) | Server `html_escape` + browser `DOMParser` + `replaceChildren`. |

The user-pain quotes that anchor the threshold defaults
(20 K Advisory, 50 K Caution) come from the
[Expensively Quadratic](https://news.ycombinator.com/item?id=47000034)
HN thread and Simon Willison's
[Too many MCPs](https://simonwillison.net/2025/Aug/22/too-many-mcps/)
post. Geoffrey Huntley's measurement that the GitHub MCP alone
"swallows another 55,000 of those valuable tokens" maps directly
to the 50 K tier.

## Known caveats

- **80 % Warning vs Anthropic's 83.5 % auto-compact**: skrills
  fires Warning at 80 % of `--alert-budget`, slightly ahead of
  Claude Code's auto-compact trigger. Community evidence
  ([anthropics/claude-code#28728](https://github.com/anthropics/claude-code/issues/28728),
  [#46695](https://github.com/anthropics/claude-code/issues/46695))
  suggests 75 % may be safer for sessions you intend to compact;
  v0.9.0 is expected to make this configurable per-tier.
- **Kill-switch override**: there is no "ignore the kill-switch"
  flag in v0.8.0. If you hit 100 %, raise `--alert-budget` and
  restart. This matches the safer-than-sorry posture of cockpit
  Warning alerts in FAA AC 25.1322-1; if it proves too restrictive
  in practice we may add an opt-in `--allow-budget-override`.
- **SSE shutdown semantics**: the browser surface merges a
  shutdown notify into the SSE response stream so `Ctrl-C` returns
  within the 2 s budget. Without the merge, a pending
  broadcast-await would block graceful shutdown indefinitely
  ([axum #2673](https://github.com/tokio-rs/axum/issues/2673),
  [hyper #2787](https://github.com/hyperium/hyper/issues/2787)).
  Future maintainers: do not "simplify" by removing the merge.

## Roadmap

- TUI mounting in the CLI (currently library code only).
- Production tick producer using `analyze::tokens::count_tokens_attributed`
  against real discovery output (replaces the demo producer).
- Per-tier configurable thresholds (community evidence supports
  75 % Warning; defer to v0.9.0).
- Clippy-style `Applicability` axis for hints (MachineApplicable /
  MaybeIncorrect / HasPlaceholders / Unspecified) orthogonal to
  severity ([rust-clippy precedent](https://github.com/rust-lang/rust-clippy/blob/master/clippy_lints/src/needless_late_init.rs)).
- ISA-18.2 ack state machine for master-acknowledge
  (Normal → Unack → Ack → RTNUnack, plus Shelved / Suppressed /
  OOS).
- gRPC service surface for external clients. The wire-format crate
  `skrills-snapshot` is already designed proto-friendly per the
  brief.

## Reference

- Brief: [`docs/cold-window-brief.md`](https://github.com/athola/skrills/blob/master/docs/cold-window-brief.md)
- Spec: [`docs/cold-window-spec.md`](https://github.com/athola/skrills/blob/master/docs/cold-window-spec.md)
- Plan: [`docs/cold-window-plan.md`](https://github.com/athola/skrills/blob/master/docs/cold-window-plan.md)
- War-room decision: [`docs/cold-window-war-room.md`](https://github.com/athola/skrills/blob/master/docs/cold-window-war-room.md)
