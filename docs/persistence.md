# Persistence and State Reporting

This document details `skrills`'s persistence mechanisms: what data is persisted, where it is stored, and how Codex or Claude Code users can inspect or reset it.

## What Gets Persisted

- **Runtime overrides**: Runtime overrides, such as `manifest_first` and `render_mode_log`, are stored in [`~/.codex/skills-runtime.json`](~/.codex/skills-runtime.json). These settings are set with the `set-runtime-options` MCP tool and override both environment variables and default manifest configurations.
- **Skill mirrors (optional)**: Skill mirrors are stored in [`~/.codex/skills-mirror/`](~/.codex/skills-mirror/). This directory contains copies of skills fetched from [`~/.claude/`](~/.claude/) when you run either the `sync-from-claude` tool or `skrills sync` command. This keeps Codex and Claude skill sources in sync without modifying the original files.
- **Pinned skills**: Pinned skills are listed in [`~/.codex/skills-pinned.json`](~/.codex/skills-pinned.json). These skills are always considered for autoloading, so they don't need to be re-read from disk on each invocation. They can be managed via CLI commands (`pin`, `unpin`, `list-pinned`) or through MCP tools (`pin-skills`, `unpin-skills`, `list-skills` with `pinned_only=true`). You can also pin skills at startup with the `SKRILLS_PINNED` environment variable (e.g., `SKRILLS_PINNED=skill-a,skill-b`), which merges them into memory without altering the persisted file.
- **Discovery cache (in-memory)**: Skill metadata is held in an in-memory discovery cache, configured with a Time-To-Live (TTL) either by `SKRILLS_CACHE_TTL_MS` or `cache_ttl_ms` within the skill manifest. This cache is volatile, meaning it is not written to disk and expires automatically or when you run the `refresh-cache` tool.
- **No user prompts**: It is important to note that user prompts and their injected context are strictly transient, exist only in memory, and are never persisted to disk.

## How Persistence Is Reported (Codex and Claude Code)

- **`runtime-status` (MCP tool)**: The `runtime-status` MCP tool shows the active runtime overrides, indicating their origin (e.g., environment variables versus runtime configuration files), and displays key operational limits. This tool can be invoked directly from within Codex or Claude Code, with its output appearing in the IDE panel.
- **`list-skills` (MCP tool)**: The `list-skills` MCP tool lists all discovered skills, detailing their respective source roots (e.g., [`~/.codex/skills`](~/.codex/skills), [`~/.codex/skills-mirror`](~/.codex/skills-mirror), or [`~/.claude/`](~/.claude/) when enabled). This allows users to confirm the specific persisted mirror content being used.
- **Pinned visibility**: The `list-skills` tool identifies pinned skills by marking them with `pinned: true`. Additionally, it supports a `pinned_only=true` parameter to display only those items that have been pinned.
- **Pin/unpin controls**: The `pin-skills` and `unpin-skills` commands manage the pinned skill set directly from Codex or Claude Code.
- **`refresh-cache` (MCP tool)**: The `refresh-cache` MCP tool clears the in-memory discovery cache. This is particularly useful after adding or editing skills, as it allows for an immediate refresh without requiring a full system restart.
- **`sync-from-claude` (MCP tool)**: The `sync-from-claude` MCP tool copies updated skills into [`~/.codex/skills-mirror`](~/.codex/skills-mirror). It provides a detailed log of which files were copied and which were skipped, and saves the mirrored skills to disk.

## Operational Tips

- To return the system to a clean state, manually remove the files [`~/.codex/skills-runtime.json`](~/.codex/skills-runtime.json) and the [`~/.codex/skills-mirror/`](~/.codex/skills-mirror/) directory. Then, run `refresh-cache` to clear the in-memory state.
- To clear only the pinned skills, execute `unpin-skills` with the parameter `{"all": true}` (when operating within Codex/Claude) or use the CLI command `skrills unpin --all`. This action removes [`skills-pinned.json`](skills-pinned.json). Any pins introduced via the `SKRILLS_PINNED` environment variable will be re-applied on the next startup.
- When debugging issues related to prompt size or content truncation, configure `render_mode_log` via `set-runtime-options`. This saves the setting, ensuring that size-related logs are generated consistently across sessions.
- In Continuous Integration (CI) or ephemeral environments, it is often more efficient to bypass skill mirroring and only use the in-memory cache. This can be achieved by not running `sync-from-claude`.
