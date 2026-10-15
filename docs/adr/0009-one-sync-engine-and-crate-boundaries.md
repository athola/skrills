# ADR 0009: One Sync Engine and Server Crate Boundaries

- Status: Accepted
- Date: 2026-10-15

## Context

Two engines copied skills between tools. `skrills_sync` (adapters plus
`SyncOrchestrator`) handled every direction except Claude to Codex skills,
which `skrills-server::sync` copied with its own 1,000-line implementation.
Each caller (CLI `sync-all`, MCP `sync-from-claude`, `setup`) had to remember
to run the second engine, and the 0.9.1 move of Codex skills to
`~/.agents/skills` had to be made in both. The routing rule in
`skrills_sync::platform_routing` encoded what callers in the higher crate did.

`skrills-server` also grew into a grab-bag: the CLI imports its `discovery`,
`app`, `config`, `setup`, `tls_auto`, `sync`, `http_transport`, `trace`,
`tool_schemas` and `api` modules. Every CLI helper placed there becomes part of
a published crate's API.

## Decision

1. `skrills_sync` is the only engine that copies skills, commands, agents,
   MCP servers, instructions and plugin assets. The Claude to Codex skills
   path runs through `CodexAdapter` and `SyncOrchestrator`; the server keeps
   only MCP request handling and calls into the sync crate.
2. The server crate's non-server modules move out over several releases:
   - skill discovery helpers to `skrills-discovery` (the single root builder
     already lives there, see the 0.9.1 changelog);
   - sync helpers to `skrills_sync`;
   - `setup` to the CLI crate, or a `skrills-setup` crate if the MCP server
     still needs it.
   The CLI then depends on `skrills-server` only to start the server.
3. Platforms are named by the `skrills_sync::Platform` enum, not strings, so
   adding a tool is a compiler-guided change.

## Status of the work

- Done in 0.9.1: decision 1 (`skrills_sync::sync_claude_skills_to_codex`
  replaced the server-side copier, which is deleted), decision 3, and the
  single skill-root builder (`skrills_discovery::skill_roots`).
- Deferred: decision 2's module moves. Each move is a public API change in
  `skrills-server` and is made in its own pull request.

## Alternatives Considered

1. **Keep both engines and document the call order.** Rejected: the 0.9.1
   review found the Codex path missed in several callers, which is the
   failure this rule invites.
2. **Move everything in one release.** Rejected: one pull request touching
   every crate boundary cannot be reviewed or reverted in parts.

## Consequences

- One place to change when a tool moves its skill directory.
- `skrills-server` loses public items as modules move; each move is listed
  as breaking in the changelog.
