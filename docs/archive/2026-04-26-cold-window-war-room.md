# War Room Decision: Cold-Window Real-Time Analysis Plan

**Session**: war-room-20260426-cold-window
**Date**: 2026-04-26
**RS**: 0.48 (Type 1 — Lightweight)
**Mode**: Lightweight (3-expert protocol)
**User directive**: explicit `--war-room` request despite RS below Full Council threshold

---

## Reversibility Assessment

| Dimension | Score | Rationale |
|-----------|-------|-----------|
| Reversal Cost | 3/5 | Adding a 9-crate-touching feature; reversible but expensive after release |
| Time Lock-In | 2/5 | Hardens after v0.8.0 ships and users adopt CLI flags |
| Blast Radius | 3/5 | All skrills users see new TUI/browser surface |
| Information Loss | 2/5 | gRPC roadmap preserved; SSE choice reversible |
| Reputation Impact | 2/5 | Open-source dev tool, not life-critical |
| **Total** | **12/25 = 0.48** | **Type 1 / Lightweight panel** |

---

## Decision Summary

**APPROVE** the cold-window implementation plan with 7 amendments
addressing Red Team challenges. Two new tasks added (TASK-031 graceful
shutdown; HTTP/2 promotion in TASK-019). Five criterion additions
across TASK-007, TASK-011, TASK-015, TASK-019, TASK-022. Plan grows
from 30 to 31 tasks; from 82 to 85 story points; realistic wall-clock
revised from 4 weeks to 5–6 weeks.

---

## Selected Approach

Bottom-up phased implementation across 9 crates:

1. **Sprint 1 (Foundation)**: workspace member + wire-format crate +
   contract traits (RED phase) + test fixtures + bounded-resource
   engine skeleton.
2. **Sprint 2 (Core)**: cold-window engine integration (GREEN phase) +
   per-source token attribution + multi-signal hint scorer +
   AlertManager-style tome dispatcher with persistence + rolling
   baselines + layered alert policy + fieldwise diff.
3. **Sprint 3 (Surfaces)**: TUI alert/hint/research/status panes with
   resize handling + HTTP/2 server endpoint + askama-templated SSE
   fragments + CLI flags + plugin participation (stretch) + graceful
   shutdown.
4. **Sprint 4 (Polish)**: parity tests + tick-budget benchmarks +
   chaos test + token attribution accuracy + adaptive cadence test +
   docs + dogfood Makefile target + final verification sweep.

Critical path: 23 story points across 8 tasks. Sprint capacity 18–26
points/week for one developer.

---

## Red Team Challenges Resolved

| ID | Challenge | Verdict | Resolution |
|---|---|---|---|
| RT-1 | Velocity claim 4 weeks unrealistic | LANDS | Plan §4 wall-clock revised to 5–6 weeks |
| RT-2 | Browser HTTP/1.1 6-conn-per-origin limit | LANDS | TASK-019 promoted to HTTP/2 + R8 added |
| RT-3 | 95% token attribution claim is fixture-gameable | LANDS | SC5 phrasing made fixture-bound, R3 updated |
| RT-4 | UX framework (FAA cockpit) misfit for AI users | DEFENDED | WARNING tier reserved for hard limits only; CHI 2025 honored |
| RT-5 | handlebars vs askama template ambiguity | LANDS | Locked to askama (compile-time, type-safe) |
| RT-6 | No graceful shutdown / SIGINT handling | LANDS | New TASK-031 added |
| RT-7 | health.toml ecosystem doesn't exist | LANDS | TASK-022 demoted to v0.8.0-stretch |
| RT-8 | Token bucket resets on restart (exploit) | LANDS | TASK-011 + R10: persistence to JSON with pro-rata refill |
| RT-9 | Terminal resize handling unspecified | LANDS | TASK-015 + R9: resize-event criterion + test |
| RT-10 | Long-running daemon memory growth | LANDS | TASK-007 + R11: bounded broadcast (16), bounded activity ring (100) |

8 of 10 challenges resulted in plan amendments. RT-4 was rebutted on
substance. RT-1 was a meta-concern absorbed into wall-clock revision.

---

## Premortem (6-Week Failure Modes)

| ID | Failure | Mitigation |
|---|---|---|
| F1 | Browser hangs on 4th tab | RT-2 fix: HTTP/2 in T019 |
| F2 | Alert fatigue → users disable feature | T013 hysteresis + min-dwell + T025 chaos test |
| F3 | Memory growth to 2GB after 8h | T007 bounded resources + R11 |
| F4 | Quota bypass via restart | T011 persistence + R10 |
| F5 | Field accuracy << 95% on real configs | SC5 honest phrasing + future tokenizer integration |
| F6 | SSE drop behind corporate proxy | T020 keep_alive() already specified |

---

## Implementation Orders

1. [ ] Apply 11 plan amendments to `docs/cold-window-plan.md` (DONE)
2. [ ] Commit brief + spec + plan + war-room decision as a single
       atomic artifact set
3. [ ] Invoke `Skill(attune:project-execution)` to begin Sprint 1
4. [ ] Execute TASK-001 (scaffold `skrills-snapshot` workspace member)
       with TDD red/green/refactor discipline per Iron Law
5. [ ] Continue through Sprint 1 with checkpoint at TASK-007
       completion
6. [ ] Hand back to user for sprint-1 review before advancing to
       Sprint 2

---

## Reversal Plan

If post-shipping data shows:

- **Cold-refresh too expensive on real workloads** → drop to
  hybrid push (file-watcher invalidation) without changing the
  snapshot contract; surfaces remain identical.
- **SSE+HTML insufficient for desired interactivity** → add WebSocket
  bidirectional surface (browser surface B from brief § 4.1) without
  removing SSE; users opt into the richer surface.
- **4-tier alert taxonomy too granular** → collapse to 2 tiers
  (critical/info) by mapping CAUTION+WARNING → critical and
  ADVISORY+STATUS → info. Single column rename in
  `WindowSnapshot::Alert`.
- **gRPC v0.9.0 follow-up infeasible** → permanently retire the
  external-client surface; no impact on v0.8.0 browser/TUI users.

---

## Dissenting Views

None this round. Red Team Commander voiced 10 challenges, 8 of which
landed and were absorbed. Chief Strategist position aligned with
Supreme Commander on retained architecture (SSE+HTML, 4-tier alerts,
allostatic thresholds).

---

## Watch Points (post-deployment)

- p99 tick duration on real ecosystems (target <200ms; alert if >500ms
  in field telemetry).
- alerts/hour on real users (target <12; alert if a user disables the
  feature within 24h of opt-in).
- HTTP/2 negotiation rate (alert if browsers fall back to HTTP/1.1
  for >5% of sessions — indicates TLS or proxy issue).
- Memory RSS growth over 24h uptime (alert if delta > 100MB).
- Quota persistence file write errors (alert at any).

---

## Session Artifacts

- `docs/cold-window-brief.md` — architecture + research (rev 2)
- `docs/cold-window-spec.md` — FRs + SCs + contracts (rev 1)
- `docs/cold-window-plan.md` — 31 tasks + 4 sprints + amendments (rev 2)
- `docs/plans/2026-04-26-war-room-cold-window.md` — this file
