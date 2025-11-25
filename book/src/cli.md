# CLI Usage

This page provides a reference for the `codex-mcp-skills` command-line interface.

## `serve`
Starts the MCP server over stdio.
```bash
codex-mcp-skills serve [--skill-dir DIR] [--cache-ttl-ms N] [--watch]
```
- `--watch`: Enables live filesystem invalidation (requires the `watch` feature flag).
- `--cache-ttl-ms`: Sets the discovery cache TTL. This can also be configured via an environment variable or the manifest file.

## `emit-autoload`
Returns concatenated skill content, plus diagnostics and structured metadata. The output is filtered by prompt, pins, auto-pins, and a byte limit.
```bash
codex-mcp-skills emit-autoload [--include-claude] [--max-bytes N] \
  [--prompt TEXT] [--auto-pin] [--skill-dir DIR]... [--diagnose]
```

## `list`
Lists all discovered skills.
```bash
codex-mcp-skills list
```

## `list-pinned`
Lists all pinned skills.
```bash
codex-mcp-skills list-pinned
```

## `pin` and `unpin`
Manage manually pinned skills.
```bash
codex-mcp-skills pin <skill>...
codex-mcp-skills unpin <skill>... [--all]
```

## `auto-pin`
Manages heuristic-based auto-pinning, which is based on usage history.
```bash
codex-mcp-skills auto-pin --enable
```

## `history`
Displays the history of autoloaded snippets.
```bash
codex-mcp-skills history [--limit N]
```

## `sync-agents`
Writes the `<available_skills>` XML block into `AGENTS.md`, including priority ranks and locations.
```bash
codex-mcp-skills sync-agents [--path AGENTS.md]
```

## `sync`
Mirrors skills from `~/.claude/skills` to `~/.codex/skills-mirror`.
```bash
codex-mcp-skills sync
```

## `tui`
Starts an interactive terminal user interface for pinning skills and optionally mirroring them.
```bash
codex-mcp-skills tui
```
