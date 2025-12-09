# State Management and Persistence

`skrills` mostly uses in-memory data, persisting only a few control files. This chapter describes the data it retains and how Codex or Claude Code users can inspect or reset this state.

## What Is Persisted

- **Runtime Overrides**: Stored in `~/.codex/skills-runtime.json`, this file saves `manifest_first` and `render_mode_log` settings from the `set-runtime-options` MCP tool. These overrides take precedence over both environment variables and manifest defaults.
- **Pinned Skills**: The file `~/.codex/skills-pinned.json` lists skills that are always eligible for autoloading, avoiding repeated disk reads. These can be managed through CLI commands (`pin`, `unpin`, `list-pinned`) or MCP tools (`pin-skills`, `unpin-skills`, `list-skills` with `pinned_only=true`). Pins can also be set at startup using the `SKRILLS_PINNED` environment variable (e.g., `SKRILLS_PINNED=skill-a,skill-b`); these merge in-memory without changing the persistent file.
- **Skill Mirrors**: Optional skill copies are stored in `~/.codex/skills-mirror/`. They are populated from `~/.claude/` (or `SKRILLS_MIRROR_SOURCE`) when `skrills mirror`, `skrills sync`, `skrills sync-all`, or the `sync-from-claude` MCP tool runs, keeping Claude and Codex skill sources aligned without changing the originals. Command files are mirrored byte-for-byte (non-UTF-8 safe) and can skip overwriting existing targets with `--skip-existing-commands`.
- **Subagent Defaults**: If present, `~/.codex/subagents.toml` sets `default_backend`, optional model overrides, and timeout defaults for `skrills agent`; `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` overrides `default_backend` at runtime.
- **Discovery Cache (In-Memory Only)**: Skill metadata resides in an in-memory cache, configured with a Time-To-Live (TTL) set by `SKRILLS_CACHE_TTL_MS` or `cache_ttl_ms` in the manifest. This cache automatically expires or can be invalidated by running `refresh-cache`.
- **Never Persisted**: User prompts and injected context are transient, existing solely in memory and never written to disk.

## How to See or Reset State (Codex & Claude Code)

- `runtime-status` (MCP tool): This tool displays effective runtime overrides and their sources. You can run it directly from the IDE tool panel to verify persistent configurations.
- `list-skills` (MCP tool): This tool marks `pinned: true` for pinned skills and accepts `pinned_only=true` to filter results to only pinned items. It also shows the source root for each skill, clarifying whether content originates from `~/.codex/skills`, `skills-mirror`, or `~/.claude/`.
- `pin-skills` / `unpin-skills` (MCP tools): These tools directly manage the pinned skill set from within Codex or Claude Code.
- `refresh-cache` (MCP tool): This tool invalidates the in-memory discovery cache, useful after adding or editing skills to ensure immediate updates.
- `sync-from-claude` (MCP tool): Populates the `skills-mirror` directory and reports on copied versus skipped files. Re-run it after Claude skill changes; CLI equivalents are `mirror`, `sync`, or `sync-all`.

## Clean Resets and Safety

- To reset runtime overrides and revert to manifest/environment defaults, delete `~/.codex/skills-runtime.json`.
- To make the system exclusively read live skill directories, remove `~/.codex/skills-mirror/` and then run `refresh-cache`.
- To clear the pinned state, run `unpin-skills {"all": true}` (or `skrills unpin --all`). This deletes `skills-pinned.json`. Pins set via the `SKRILLS_PINNED` environment variable will be re-established on the next startup unless the environment variable is removed.
- Maintain `render_mode_log` enabled via `set-runtime-options` when auditing truncation. Size diagnostics will be emitted in every session until explicitly disabled.
