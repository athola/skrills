# Tome Research — TUI + Browser Real-Time Dashboard Prior Art

> Captured 2026-04-26 during cold-window v0.8.0 polish.
> Source: tome:code-searcher agent on GitHub at the request of
> the user (`/attune:mission` continuation, "do deep research
> online using the tome plugin for other projects on github or
> hackernews").

## TL;DR

Across 10 reference projects, the dominant pattern for shipping
a TUI and a browser dashboard from one engine is a **shared,
lock-protected snapshot type** consumed by two thin renderers —
exactly skrills' `analyze::engine` -> `(tui, browser)` shape.
Vector's `vector top` (250 ms throttle on 500 ms polls), ccboard
(file-watcher debounce + SSE), and Glances (plugin core, two
presenters) all converge on this. For tick scheduling, ratatui
templates universally separate **tick rate** (state advance)
from **frame rate** (redraw) and merge them with `tokio::select!`;
redraw-on-dirty saves an order of magnitude of CPU. Cold rewalks
are well-precedented (Prometheus `file_sd`, fluent-bit stat
watcher) provided the rewalk fits inside the refresh budget —
the same constraint skrills enforces with its <200 ms p99 budget.
SSE on axum has a known footgun: pending broadcast streams block
graceful shutdown (axum issues #2673, #2787); the fix is to merge
the shutdown signal into the stream and apply a `TimeoutLayer`.

## 1. TUI + browser parity

| Project | Verdict | Notes |
|---|---|---|
| [FlorianBruniaux/ccboard](https://github.com/FlorianBruniaux/ccboard) | **Validates** | `ccboard-core` exposes a thread-safe `DataStore`; ccboard-tui (ratatui) and ccboard-web (axum + Leptos) both consume it. File-watcher 500 ms debounce + SSE for web. Mirrors skrills' engine/surface split. |
| [njbrake/agent-of-empires](https://github.com/njbrake/agent-of-empires) | **Inspires** | UI-agnostic core with state behind tmux/session persistence; web speaks WebSocket relay, TUI direct. Suggests skrills could expose a structured snapshot stream others can subscribe to. |
| [nicolargo/glances](https://github.com/nicolargo/glances) | **Validates** | Plugin-based collector core; curses and web UI are sibling presenters reading the same plugin output. Encourages skrills to keep alert/hint/token attribution as plugin-shaped functions. |
| [vectordotdev/vector PR #4702](https://github.com/vectordotdev/vector/pull/4702) | **Validates** | `Arc<Mutex<BTreeMap<String, TopologyRow>>>` is the canonical Rust shape for a snapshot map shared between renderers. |

Common parity boundary: a single immutable snapshot struct
produced per tick. Neither protobuf nor an actor framework was
needed in any of these; an `Arc<Mutex<…>>` snapshot was sufficient.

## 2. Tick-driven refresh with adaptive cadence

- [ratatui/async-template](https://github.com/ratatui/async-template)
  splits `--tick-rate` (default 1 Hz state advance) from
  `--frame-rate` (default 60 Hz redraw) and merges Tick / Render /
  Input via `tokio::select!`. The README explicitly recommends a
  **dirty flag** so redraws happen only when state changes.
  **Verdict: validates** skrills' two-track scheduler.
- [vectordotdev/vector top](https://github.com/vectordotdev/vector/pull/4702)
  demonstrates an **adaptive throttle**: 500 ms poll, 250 ms
  redraw cap. Cut CPU from ~100 % to 2-3 %. **Verdict: inspires**
  — skrills could document the throttle separately from the
  rewalk interval.
- [ratatui-website Discussion #89](https://github.com/ratatui/ratatui-website/discussions/89)
  "CPU usage is too high" — canonical advice: never redraw on a
  busy loop; gate on event or change. **Verdict: validates**.

## 3. Cold rewalk vs hot cache

- [prometheus/prometheus file_sd / http_sd](https://github.com/prometheus/prometheus/blob/main/docs/http_sd.md)
  does a full re-read every `refresh_interval` (5 m / 1 m default).
  Issues [#4301](https://github.com/prometheus/prometheus/issues/4301)
  and [#6327](https://github.com/prometheus/prometheus/issues/6327)
  show what happens when refresh starts to contend with scrape
  budget. **Verdict: validates** the cold-rewalk model but
  **warns** that the rewalk must finish well inside the tick
  budget at p99, not p50. Skrills' 200 ms target on 200-500
  entities is in line with Prometheus' file_sd at scale.
- [fluent/fluent-bit in_tail](https://github.com/fluent/fluent-bit/blob/master/plugins/in_tail/tail.c)
  ships both inotify and a stat-based polled watcher with a
  configurable `Refresh_Interval`. **Verdict: inspires** —
  skrills could document a fallback "fast path" using metadata
  mtime gates if rewalk ever blows the budget at >1 k entities.

## 4. SSE-driven HTMX-style dashboards in Rust

- [tokio-rs/axum SSE example](https://github.com/tokio-rs/axum/blob/main/examples/sse/src/main.rs):
  `Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(1)).text("keep-alive-text"))`
  is the canonical pattern. **Verdict: validates**.
- [tokio-rs/axum graceful-shutdown example](https://github.com/tokio-rs/axum/blob/main/examples/graceful-shutdown/src/main.rs):
  `axum::serve(listener, app).with_graceful_shutdown(shutdown_signal())`
  + `TimeoutLayer`, with `tokio::select!` on `ctrl_c` and
  `SIGTERM`. **Verdict: validates**.
- [axum issue #2673](https://github.com/tokio-rs/axum/issues/2673)
  and [hyperium/hyper #2787](https://github.com/hyperium/hyper/issues/2787):
  an SSE stream awaiting a broadcast channel will block graceful
  shutdown indefinitely if no event arrives. Recommended fix:
  merge a `shutdown.notified()` future into the stream so the
  SSE response terminates cleanly with a 204 (so HTMX doesn't
  auto-reconnect). **Verdict: contradicts** any naive "just await
  broadcast" implementation — this is a load-bearing footgun
  skrills should call out in docs.
- [robertwayne/axum-htmx](https://github.com/robertwayne/axum-htmx):
  typed extractors for HX-Request / HX-Swap / HX-Trigger headers.
  **Verdict: inspires** — clean way to express skrills'
  fragment-swap endpoints.

## Skrills implications (proposed additions to `book/src/cold-window.md`)

1. **"How parity works" section** — state explicitly that
   `analyze::engine` produces a single snapshot struct per tick,
   and both surfaces are pure renderers over that snapshot. Cite
   ccboard, vector top, and Glances as prior art so users trust
   the model. (Maps to pattern 1.)
2. **"Tick budget vs redraw budget" hint** — separate the
   configurable rewalk interval from the redraw cap. Recommend
   a default like 1 Hz rewalk / 4 Hz redraw cap with dirty-flag
   gating, matching vector top's 500 ms / 250 ms ratio. (Maps to
   pattern 2.)
3. **"What happens at >500 entities" guidance** — document the
   p99 budget like Prometheus does, and explain that exceeding
   the rewalk budget will skip ticks rather than queueing them
   (so users see a stale-by-one-tick banner, never an unbounded
   queue). Suggest a future `--shallow-rewalk` escape hatch a la
   fluent-bit's stat watcher. (Maps to pattern 3.)
4. **"SSE shutdown semantics" callout** — explicitly document
   that the browser surface merges a shutdown notify into the
   SSE stream so `Ctrl-C` returns within `TimeoutLayer` bounds,
   and that the response ends with a final event so HTMX clients
   don't loop-reconnect. Reference axum issue #2673 so future
   maintainers don't "fix" the shutdown by removing the merge.
   (Maps to pattern 4.)
5. **"Why we don't cache between ticks" rationale** —
   one-paragraph justification: the ecosystem is small enough
   that a cold rewalk is cheaper than cache invalidation, and
   the simplicity benefit (no stale-cache bugs, snapshots
   always reflect disk) is what makes alert hysteresis
   trustworthy. Cite Prometheus file_sd as the precedent for
   "cold every tick is fine when the budget fits".

## Sources

- ccboard — https://github.com/FlorianBruniaux/ccboard
- agent-of-empires — https://github.com/njbrake/agent-of-empires
- glances — https://github.com/nicolargo/glances
- vector top PR — https://github.com/vectordotdev/vector/pull/4702
- ratatui/async-template — https://github.com/ratatui/async-template
- ratatui CPU discussion — https://github.com/ratatui/ratatui-website/discussions/89
- prometheus http_sd docs — https://github.com/prometheus/prometheus/blob/main/docs/http_sd.md
- prometheus #4301 (config reload perf) — https://github.com/prometheus/prometheus/issues/4301
- prometheus #6327 (reload delays scraping) — https://github.com/prometheus/prometheus/issues/6327
- fluent-bit in_tail — https://github.com/fluent/fluent-bit/blob/master/plugins/in_tail/tail.c
- fluent-bit in_tail inotify watcher — https://github.com/fluent/fluent-bit/blob/master/plugins/in_tail/tail_fs_inotify.c
- axum SSE example — https://github.com/tokio-rs/axum/blob/main/examples/sse/src/main.rs
- axum graceful-shutdown example — https://github.com/tokio-rs/axum/blob/main/examples/graceful-shutdown/src/main.rs
- axum #2673 (SSE blocks graceful shutdown) — https://github.com/tokio-rs/axum/issues/2673
- hyper #2787 (graceful shutdown w/ open SSE) — https://github.com/hyperium/hyper/issues/2787
- axum-htmx — https://github.com/robertwayne/axum-htmx
