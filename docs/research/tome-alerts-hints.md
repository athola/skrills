# Tome Research — Alert Hygiene + Hint Engine Prior Art

> Captured 2026-04-26 during cold-window v0.8.0 polish.
> Source: tome:code-searcher agent on GitHub at the request of
> the user (`/attune:mission` continuation, "do deep research
> online using the tome plugin for other projects on github or
> hackernews").

## TL;DR for the skrills cold-window v0.8.0 design review

**Section 1 — Alert hygiene (storm prevention).** Prometheus
Alertmanager's `dispatch.go` `aggrGroup` with
`group_wait` / `group_interval` / `repeat_interval` is the
canonical pattern; vmalert adds parse-time dedup of duplicate
rule definitions and recording rules as pre-aggregation; alerta
encodes the full ISA-18.2 ack state machine
(Normal / Unack / Ack / RTNUnack / Shelved / Suppressed / OOS);
Sensu uses a templated `dedup-key` so downstream incident-tools
collapse repeats. **Skrills validates** on hysteresis + min-dwell,
**should adopt** named `group_wait` / `group_interval` /
`repeat_interval` knobs, an ISA-18.2-style ack state machine for
master-acknowledge, and a Go-template-style dedup key for the
research dispatcher.

**Section 2 — Hint engines.** rust-clippy's four-level
`Applicability`
(MachineApplicable / MaybeIncorrect / HasPlaceholders / Unspecified)
and Ruff's safe-vs-unsafe split with `extend-safe-fixes` are the
cleanest precedents. ESLint validates that advisory ("warn") tier
must not affect exit code. Semgrep adds reachability filtering
and confidence-weighted auto-triage. **Skrills validates** on the
recency × severity × user-impact scoring, **should adopt** an
explicit Applicability axis orthogonal to severity, and a
"reachability"-style filter (is this hint actionable in the
current pane?).

**Section 3 — Master-acknowledge UX.** k9s configures hotkeys
via `$XDG_CONFIG_HOME/k9s/hotkeys.yaml`; lazygit binds Esc as a
context-sensitive cancel/return that takes precedence locally
over global, and shows context-aware labels ("Abort rebase:
\<esc\>"). The pattern is: one well-known key, label changes per
pane to advertise current consequence. **Skrills should adopt**
a single Esc-style master-ack that mutes the entire alert
cascade for `silence_for` minutes, with a context-aware
status-bar label so the user always sees what Esc will do
*right now*.

**Section 4 — Token-bucket persistence.** governor's GCRA stores
state as one `AtomicU64` + last-update timestamp — trivial to
snapshot to disk on graceful shutdown and rehydrate on boot
(skrills already has graceful shutdown per T031). udoprog's
leaky-bucket coordinator-free design is more featureful (fair
scheduling) but heavier; governor is the right pick for a
low-QPS research dispatcher. Persistence is your code, not the
crate's: serialize `(state_u64, last_update_unix_nanos)` to
JSON next to the quota file.

## Findings table (verdict per pattern)

| Source | Pattern | Verdict |
|---|---|---|
| [prometheus/alertmanager `dispatch.go`](https://github.com/prometheus/alertmanager/blob/main/dispatch/dispatch.go) | `aggrGroup` + `group_wait` / `group_interval` / `repeat_interval` + inhibition rules | **Validates** skrills hysteresis; **adopt** named knobs |
| [VictoriaMetrics vmalert](https://github.com/VictoriaMetrics/VictoriaMetrics/blob/master/app/vmalert/main.go) | parse-time dedup + recording rules as pre-aggregation | **Adopt** recording-rule pattern for cold-window severity rollups |
| [alerta ISA-18.2](https://github.com/alerta/alerta/blob/master/alerta/models/alarms/isa_18_2.py) | full ack state machine (Normal -> Unack -> Ack -> RTNUnack, plus Shelved / Suppressed / OOS) | **Validates** 4-tier model; **adopt** vocabulary verbatim |
| [sensu-pagerduty-handler](https://github.com/sensu/sensu-pagerduty-handler/blob/main/main.go) | `dedup-key-template` Go template | **Adopt** for research dispatcher dedupe |
| [rust-clippy Applicability](https://github.com/rust-lang/rust-clippy/blob/master/clippy_lints/src/needless_late_init.rs) | four-level Applicability tag, downgraded when comments present | **Adopt** Applicability orthogonal to severity |
| [astral-sh/ruff safe-vs-unsafe](https://github.com/astral-sh/ruff/blob/main/docs/faq.md) | safe-by-default fix; `extend-safe-fixes` / `--unsafe-fixes` opt-in | **Adopt** per-hint applicability + extend-config |
| [eslint severity](https://github.com/eslint/eslint/issues/14679) | warn tier excluded from exit code | **Validates** CAUTION as non-fatal |
| [boinkor-net/governor](https://github.com/boinkor-net/governor) | GCRA atomic state, no background task | **Adopt** for research-dispatcher quota persistence |

## Skrills follow-up: 5 hint corpus entries to ship in v0.8.0 docs

1. **If** cold-window CAUTION fires < min_dwell after previous
   CAUTION on same symbol **then** suggest "raise hysteresis
   floor by 5%; you are flapping near the trigger boundary"
   (applicability: MachineApplicable via config patch).
2. **If** research dispatcher token bucket drains > 80% in
   < group_interval **then** suggest "you are storming the LLM
   API; widen group_wait or add an inhibition rule for low-tier
   alerts" (applicability: MaybeIncorrect — needs human review
   of which tier to inhibit).
3. **If** WARNING tier alert resolves and re-fires within
   repeat_interval **then** suggest "this is a chattering
   signal; add a dead-band to the trigger or shelve for N
   minutes per ISA-18.2" (applicability: MachineApplicable —
   emit shelve command).
4. **If** EMERGENCY fires while CRITICAL on same symbol is
   unacked **then** suggest "inhibit the CRITICAL — emergency
   supersedes; press \<Esc\> to master-ack the cascade"
   (applicability: MachineApplicable, surfaced inline with the
   keystroke).
5. **If** hint pane shows > 7 active hints **then** suggest
   "you are over the operator span-of-control limit
   (ISA-18.2 §6.4); shelve advisories or raise the CAUTION
   floor" (applicability: Unspecified — operator judgement).

## Sources

- [prometheus/alertmanager dispatch.go](https://github.com/prometheus/alertmanager/blob/main/dispatch/dispatch.go)
- [prometheus/alertmanager configuration docs](https://github.com/prometheus/alertmanager/blob/main/docs/configuration.md)
- [VictoriaMetrics vmalert main.go](https://github.com/VictoriaMetrics/VictoriaMetrics/blob/master/app/vmalert/main.go)
- [VictoriaMetrics vmalert docs](https://github.com/VictoriaMetrics/VictoriaMetrics/blob/master/docs/victoriametrics/vmalert.md)
- [grafana/oncall alert groups API](https://github.com/grafana/oncall/blob/dev/docs/sources/oncall-api-reference/alertgroups.md)
- [alerta ISA-18.2 model](https://github.com/alerta/alerta/blob/master/alerta/models/alarms/isa_18_2.py)
- [alerta ISA-18.2 support issue](https://github.com/alerta/alerta/issues/1611)
- [sensu-aggregate-check](https://github.com/sensu/sensu-aggregate-check)
- [sensu-pagerduty-handler dedup-key-template](https://github.com/sensu/sensu-pagerduty-handler/blob/main/main.go)
- [semgrep/semgrep](https://github.com/semgrep/semgrep)
- [rust-lang/rust-clippy](https://github.com/rust-lang/rust-clippy)
- [clippy needless_late_init.rs (Applicability example)](https://github.com/rust-lang/rust-clippy/blob/master/clippy_lints/src/needless_late_init.rs)
- [clippy PR #13940 — drop MachineApplicable when comments present](https://github.com/rust-lang/rust-clippy/pull/13940)
- [clippy Applicability discussion #9994](https://github.com/rust-lang/rust-clippy/discussions/9994)
- [astral-sh/ruff faq.md (safe vs unsafe fixes)](https://github.com/astral-sh/ruff/blob/main/docs/faq.md)
- [ruff issue #4181 — safe vs unsafe fixes](https://github.com/astral-sh/ruff/issues/4181)
- [eslint configure rules / severity model](https://github.com/eslint/eslint/issues/14679)
- [eslint PR #985 — store severity on message](https://github.com/eslint/eslint/pull/985)
- [boinkor-net/governor](https://github.com/boinkor-net/governor)
- [udoprog/leaky-bucket](https://github.com/udoprog/leaky-bucket)
- [Gelbpunkt/leaky-bucket-lite](https://github.com/Gelbpunkt/leaky-bucket-lite)
- [derailed/k9s](https://github.com/derailed/k9s)
- [jesseduffield/lazygit Config.md](https://github.com/jesseduffield/lazygit/blob/master/docs/Config.md)
- [lazygit PR #4819 — context-aware Esc label](https://github.com/jesseduffield/lazygit/pull/4819)
