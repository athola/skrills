# ADR 0010: Token Analysis Without the Intelligence Crate

- Status: Accepted
- Date: 2026-10-15

## Context

`skrills-analyze` counts tokens and runs the cold-window engine. The engine
defines a `HintScorer` trait, and its default constructors wire in
`skrills_intelligence::cold_window_hints::MultiSignalScorer`. That one import
makes every user of token counting compile `skrills-intelligence` and its
dependencies (reqwest, tokio process support, chrono, trigram matching).

## Decision

The intelligence-backed scorer is behind a default-on cargo feature of
`skrills-analyze`, `intelligence-scorer`. Callers that only count tokens
set `default-features = false` and do not build `skrills-intelligence`.
With the feature off, the engine's default constructors fall back to a
scorer that keeps hint order, and callers can still pass their own
`HintScorer`.

## Alternatives Considered

1. **Move the cold-window engine into its own crate.** Rejected for now:
   it splits a module that has one consumer (the CLI) and adds a publish
   step. Revisit if a second consumer appears.
2. **Leave the dependency.** Rejected: token counting is used by the
   validator and the dashboard, which have no use for network clients.

## Consequences

- Default builds behave as before.
- A build with `--no-default-features` gets plain hint ordering in the
  cold-window view.
