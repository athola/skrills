# CLI Usage Reference

This section is a reference for the command-line interface (CLI) usage of `skrills`, listing the subcommands and their functions.

## `serve`
Starts the Machine-Readable Context Protocol (MCP) server, operating over standard I/O (stdio).
```bash
skrills serve [--skill-dir DIR] [--cache-ttl-ms N] [--watch]
```
- `--watch`: Activates live filesystem invalidation (requires the `watch` compilation flag).
- `--cache-ttl-ms`: Sets the Time-To-Live (TTL) for the discovery cache. This setting can also be managed via an environment variable or within the manifest file.

## `emit-autoload`
Generates skill content, filtered based on the provided prompt, manual pins, auto-pins, and an enforced byte limit.
```bash
skrills emit-autoload [--include-claude] [--max-bytes N] \
  [--prompt TEXT] [--auto-pin] [--skill-dir DIR]... [--diagnose]
```

## `list`
Lists all currently discovered skills.
```bash
skrills list
```

## `list-pinned`
Lists all skills that have been explicitly pinned.

The `list-skills` MCP tool complements this by marking entries with `pinned: true` and supports a `pinned_only=true` filter to show only pinned skills within Codex/Claude Code environments.

## `pin` and `unpin`
These commands manage manually pinned skills.
```bash
skrills pin <skill>...
skrills unpin <skill>... [--all]
```

You can set pins at startup by setting the `SKRILLS_PINNED` environment variable (e.g., `SKRILLS_PINNED=skill-a,skill-b`). These environment-defined pins are merged with the stored set and become visible through `list-skills` and `list-pinned`.

## `auto-pin`
Manages heuristic-based auto-pinning, which dynamically pins skills based on their usage history.
```bash
skrills auto-pin --enable
```

## `history`
Shows a historical record of all autoloaded snippets.
```bash
skrills history [--limit N]
```

## `sync-agents`
Generates and writes the `<available_skills>` XML block into [`AGENTS.md`](AGENTS.md), including priority ranks and their respective locations.
```bash
skrills sync-agents [--path AGENTS.md]
```

## `mirror`
Mirrors Claude assets (skills, agents, commands, and MCP prefs) into the Codex defaults and refreshes `AGENTS.md`.
```bash
skrills mirror [--dry-run] [--skip-existing-commands]
```
- `--dry-run` hashes sources and reports intended writes without changing files.
- `--skip-existing-commands` preserves any prompts already present under `~/.codex/prompts`.

## `sync`
Mirrors skills from the `~/.claude/skills` directory to the `~/.codex/skills-mirror` directory.
```bash
skrills sync [--skip-existing-commands]
```
Honors `SKRILLS_MIRROR_SOURCE` to change the source root (e.g., when Claude content lives elsewhere) and will avoid overwriting existing commands when `--skip-existing-commands` is set.

## `sync-commands`
Syncs slash commands between Claude Code and Codex.
```bash
skrills sync-commands [--from claude|codex] [--dry-run] [--skip-existing-commands]
```
- `--from`: Source side (default `claude`).
- `--dry-run`: Preview changes.
- `--skip-existing-commands`: Do not overwrite commands already present on the target.
- Commands are copied byte-for-byte so non-UTF-8 command files are mirrored without re-encoding.

## `sync-mcp-servers`
Syncs MCP server configurations between Claude Code and Codex.
```bash
skrills sync-mcp-servers [--from claude|codex] [--dry-run]
```

## `sync-preferences`
Syncs user preferences between Claude Code and Codex.
```bash
skrills sync-preferences [--from claude|codex] [--dry-run]
```

## `sync-all`
Runs skills mirror plus command, MCP server, and preference syncs in one pass.
```bash
skrills sync-all [--from claude|codex] [--dry-run] [--skip-existing-commands]
```
- `--skip-existing-commands`: Mirror skills and metadata but keep any commands already present on the target side.

## `sync-status`
Shows sync status and configuration deltas.
```bash
skrills sync-status [--from claude|codex]
```

## `agent`
Launches a discovered agent by name using the stored run template.
```bash
skrills agent <name> [--skill-dir DIR]... [--dry-run]
```
Use `--dry-run` to print the resolved command without executing it.
When no backend is specified in an agent spec, skrills checks `~/.codex/subagents.toml` for a `default_backend`; if absent it falls back to `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` and defaults to `codex`.
Command sync is byte-for-byte, so non-UTF-8 command files remain intact.

## `doctor`
Diagnoses Codex MCP configuration for this server.
```bash
skrills doctor
```

## `tui`
Launches an interactive Terminal User Interface (TUI) for pinning skills and optionally managing their mirroring.
```bash
skrills tui [--skill-dir DIR]...
```

## `setup`
Configures skrills for Claude Code or Codex (hooks, MCP entries, directories).
```bash
skrills setup [--client CLIENT] [--bin-dir DIR] [--reinstall] [--uninstall] [--add] [-y|--yes] [--universal] [--mirror-source DIR]
```
- `--client`: Target (`claude`, `codex`, or `both`).
- `--bin-dir`: Override install location.
- `--reinstall` / `--uninstall` / `--add`: Control lifecycle.
- `--yes`: Non-interactive mode.
- `--universal`: Also mirror skills to `~/.agent/skills`.
- `--mirror-source`: Override source directory for mirroring (default `~/.claude`).

## MCP Tools (Client-Facing)
The `skrills` server exposes these client-facing tools via the MCP protocol:
- `list-skills`: Lists all discovered skills, providing their source and hash metadata.
- `autoload-snippet`: Generates skill content filtered by the user's prompt (with a manifest-first approach by default).
- `runtime-status`: Displays the effective `manifest_first` and `render_mode_log` values, with their respective override sources.
- `set-runtime-options`: Updates runtime overrides, which are persistently stored in [`~/.codex/skills-runtime.json`](~/.codex/skills-runtime.json).
- `render-preview`: Offers a preview of matched skill names, the manifest's byte size, and an estimated token count, without returning the full skill content. This tool is useful for inspecting or checking payloads before injecting `additionalContext`.
- `refresh-cache`, `sync-from-claude`: These commands are for cache maintenance and synchronization of Claude skills.

Example of `render-preview` tool usage:
```
{
  "name": "render-preview",
  "arguments": {
    "prompt": "harden api error handling",
    "embed_threshold": 0.25,
    "auto_pin": true
  }
}

Structured content (trimmed):
{
  "matched": ["api-review/SKILL.md", "defense-in-depth/SKILL.md"],
  "manifest_bytes": 1824,
  "estimated_tokens": 456,
  "truncated": false,
  "truncated_content": false
}
```
