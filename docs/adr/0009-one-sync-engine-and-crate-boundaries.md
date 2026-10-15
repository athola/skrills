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
- Also done in 0.9.1 (decision 2), each listed as breaking in the changelog:
  - `skrills_server::discovery` helpers moved to `skrills_discovery` (module
    `lookup`; the env-driven `skill_roots` is now `skill_roots_from_env`).
    The `doc://agents` resource constants stay private to the server.
  - `skrills_server::sync` moved to `skrills_sync` (module `agents_md`),
    with the default AGENTS.md template; `skrills_sync` now depends on
    `skrills-discovery`, which does not depend on it.
  - `skrills_server::setup` became the `skrills-setup` crate, since the
    server still calls `is_setup`.
  - `skrills_server::config` (the `~/.skrills/config.toml` loader) moved to
    the CLI crate; the server never read it.
- Left in `skrills-server`, used by the CLI to start the server: `app`
  (`SkillService`, `start_fs_watcher`), `http_transport`, `tls_auto`
  (`skrills cert` shares the server's certificate generator), `trace`,
  `tool_schemas`, `mcp_gateway` and `api` (cold-window HTTP routes).
- Not yet done: `recommend`, `metrics`, `resolve`, `skill-uri` and the
  intelligence commands run `SkillService` and its ranking/metrics helpers
  offline. Moving them out means separating the engine from the MCP
  handler, a larger change than a module move.

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
