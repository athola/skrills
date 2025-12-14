# ADR 0001: Pivot to Support Engine (v0.3.1)

- Status: Accepted
- Date: 2025-12-14

## Context

Version 0.3.1 shifted `skrills` from a thin CLI helper to a support engine that fronts multiple agent ecosystems (Codex, Claude) via MCP. The change expanded scope to include skill discovery, synchronization across agent runtimes, and command handlers for mirroring agents and commands.

## Decision

- Centralize runtime orchestration in the `app` module while delegating user-facing commands to `commands/*`.
- Keep MCP server responsibilities in `SkillService`, exposing skills, agents, and validation tooling.
- Maintain backwards compatibility for CLI users while enabling the support engine to serve as a shared backend for multiple agents.

## Consequences

- Broader dependency surface (sync, discovery, validation) requires clearer module ownership and documentation.
- Future features should be documented as ADRs before implementation to preserve architectural intent.
- Module boundaries may need tightening (e.g., further splitting `app.rs` if it grows past 2500 LOC).
