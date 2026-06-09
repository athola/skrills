# ADR 0007: Cold-Window Real-Time Analysis Architecture (v0.8.0)

- Status: Accepted
- Date: 2026-04-26

## Context

v0.8.0 adds a continuously-refreshing analysis surface ("cold window") that
monitors token usage, hint signals, plugin health, and external research in
real time. Two render targets were required: a ratatui TUI for terminal users
and a browser surface (SSE and HTML fragments) for remote/shared access.

The design had to satisfy competing constraints: disk-state freshness (no
warm-cache shortcuts), bounded memory growth in long-running daemon mode,
multi-browser concurrency, and graceful kill-switch behavior on token-budget
exhaustion.

## Decision

**Single snapshot fan-out over a bounded broadcast channel.**
`ColdWindowEngine` emits one `Arc<WindowSnapshot>` per tick. Both render
targets are pure consumers of the same artifact; TUI ↔ browser parity is
structurally enforced rather than tested by convention.

**Cold filesystem rewalk per tick.**
Authoritative state is always read from disk. No cache invalidation logic,
no staleness window. This is only viable because the SC1 budget (p99 < 200 ms
per tick) fits the current ecosystem scale; Prometheus `file_sd` uses the
same pattern.

**SSE and server-side HTML fragments for the browser surface.**
WebSocket and gRPC were evaluated and deferred. SSE is one-way (sufficient
for this use case), HTTP/2-multiplexable (solves the 6-connection-per-origin
limit without client-side changes), and forward-compatible with the v0.9.0
gRPC roadmap. The wire-format crate (`skrills-snapshot`) is proto3-compatible.

**4-tier alert model (Warning/Caution/Advisory/Status).**
Maps to FAA AC 25.1322-1 cockpit CAS. Warning-tier is reserved for limits
that require immediate user action; the kill-switch only engages at Warning.
CHI 2025 evidence on alarm fatigue is honored by keeping Caution and below
panel-only with hysteresis and min-dwell.

**Research dispatcher as an AlertManager-style token-bucket.**
External fetches are quota-gated and restart-resilient via JSON persistence
at `~/.skrills/research-quota.json`. Prevents research storms under session
churn.

**HTTP/2 promotion via TLS (ALPN `h2`).**
Multiple browser tabs in the same origin stay subscribed past HTTP/1.1's
6-connection limit through stream multiplexing.

## Rationale

Key red-team challenges resolved before implementation:

| Challenge | Resolution |
|---|---|
| HTTP/1.1 6-connection browser limit | HTTP/2 via axum-server and rustls ALPN |
| Token-bucket bypass via restart | Persist quota to JSON, refill pro-rata on load |
| No graceful SIGINT handling | 2 s cleanup budget; SSE merges shutdown notify |
| Memory growth in daemon mode | Broadcast channel cap 16; activity ring cap 100 |
| Alert fatigue | Hysteresis clear ratio 0.95, min-dwell 2 ticks, 4-tier model |

## Reversal paths

- **Cold-refresh too expensive on real workloads** → adopt push (file-watcher
  invalidation) without changing the snapshot contract; surfaces are
  unaffected.
- **SSE insufficient for interactivity** → add WebSocket surface as an opt-in
  alongside SSE; no removal required.
- **4-tier taxonomy too granular** → collapse Warning and Caution into
  critical, Advisory and Status into info. Single enum rename; snapshot
  contract otherwise unchanged.
- **gRPC v0.9.0 infeasible** → retire external-client surface; v0.8.0 users
  are unaffected.

## Watch points

- p99 tick duration on real ecosystems: target < 200 ms; alert field
  telemetry if > 500 ms.
- Alerts-per-hour on active users: target < 12; feature is likely too noisy
  if a user disables within 24 h of opt-in.
- HTTP/2 negotiation rate: alert if browsers fall back to HTTP/1.1 for > 5%
  of sessions (TLS or proxy issue).
- Memory RSS growth over 24 h: alert if delta > 100 MB.
- Quota persistence write errors: alert on any failure.

## Consequences

- `skrills-snapshot` is a new wire-format crate; breaking changes follow
  semver and require incrementing the `version` field in `WindowSnapshot`.
- The broadcast channel bus creates a hard dependency ordering:
  engine → analyze crate → snapshot crate. Consumers must not take snapshots
  crate as a direct dependency on the engine.
- The `http-transport` feature flag gates all browser-surface code; builds
  with `--no-default-features` are unaffected.
