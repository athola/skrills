# CLI Usage Reference

This section is a reference for the command-line interface (CLI) usage of `skrills`, listing the subcommands and their functions.

## `serve`

Starts the MCP server, operating over standard I/O (stdio).

```bash
skrills serve [--skill-dir DIR] [--cache-ttl-ms N] [--watch]
```

- `--watch`: Activates live filesystem invalidation (requires the `watch` compilation flag).
- `--cache-ttl-ms`: Sets the Time-To-Live (TTL) for the discovery cache.

## `validate`

Validates skills for Claude Code and/or Codex CLI compatibility.

```bash
skrills validate [OPTIONS]
```

Options:
- `--skill-dir <DIR>`: Skills directory to validate (default: all discovered skills)
- `--target <TARGET>`: Validation target: `claude`, `codex`, or `both` (default: `both`)
- `--autofix`: Automatically fix validation issues (add missing frontmatter)
- `--backup`: Create backup files before autofix
- `--format <FORMAT>`: Output format: `text` or `json` (default: `text`)
- `--errors-only`: Only show skills with errors

Examples:
```bash
skrills validate --target codex              # Check Codex compatibility
skrills validate --target codex --autofix    # Auto-add missing frontmatter
skrills validate --format json --errors-only # CI-friendly output
```

## `analyze`

Analyzes skills for token usage, dependencies, and optimization suggestions.

```bash
skrills analyze [OPTIONS]
```

Options:
- `--skill-dir <DIR>`: Skills directory to analyze (default: all discovered skills)
- `--format <FORMAT>`: Output format: `text` or `json` (default: `text`)
- `--min-tokens <N>`: Only show skills exceeding this token count
- `--suggestions`: Include optimization suggestions

Examples:
```bash
skrills analyze --min-tokens 1000            # Find large skills
skrills analyze --suggestions                # Get optimization tips
skrills analyze --format json                # Machine-readable output
```

## `sync`

Copies skills from `~/.claude` into `~/.codex/skills-mirror`.

```bash
skrills sync [--skip-existing-commands]
```

Honors `SKRILLS_MIRROR_SOURCE` to change the source root.

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

## `sync-agents`

Generates and writes the `<available_skills>` XML block into `AGENTS.md`, including priority ranks and their respective locations.

```bash
skrills sync-agents [--path AGENTS.md]
```

## `mirror`

Mirrors Claude assets (skills, agents, commands, and MCP prefs) into the Codex defaults and refreshes `AGENTS.md`.

```bash
skrills mirror [--dry-run] [--skip-existing-commands]
```

- `--dry-run`: Hashes sources and reports intended writes without changing files.
- `--skip-existing-commands`: Preserves any prompts already present under `~/.codex/prompts`.

## `agent`

Launches a discovered agent by name using the stored run template.

```bash
skrills agent <name> [--skill-dir DIR]... [--dry-run]
```

Use `--dry-run` to print the resolved command without executing it.

When no backend is specified in an agent spec, skrills checks `~/.codex/subagents.toml` for a `default_backend`; if absent it falls back to `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` and defaults to `codex`.

## `doctor`

Diagnoses Codex MCP configuration for this server.

```bash
skrills doctor
```

## `tui`

Launches an interactive Terminal User Interface (TUI) for sync management.

```bash
skrills tui [--skill-dir DIR]...
```

## `setup`

Configures skrills for Claude Code or Codex (hooks, MCP entries, directories).

```bash
skrills setup [OPTIONS]
```

Options:
- `--client`: Target (`claude`, `codex`, or `both`).
- `--bin-dir`: Override install location.
- `--reinstall` / `--uninstall` / `--add`: Control lifecycle.
- `--yes`: Non-interactive mode.
- `--universal`: Also mirror skills to `~/.agent/skills`.
- `--mirror-source`: Override source directory for mirroring (default `~/.claude`).

## MCP Tools (Client-Facing)

The `skrills` server exposes these tools via the MCP protocol:

| Tool | Description |
|------|-------------|
| `sync-from-claude` | Copy Claude skills into Codex mirror |
| `sync-skills` | Sync skills between agents |
| `sync-commands` | Sync slash commands between agents |
| `sync-mcp-servers` | Sync MCP server configurations |
| `sync-preferences` | Sync preferences between agents |
| `sync-all` | Sync all configurations |
| `sync-status` | Preview sync changes (dry run) |
| `validate-skills` | Validate skills for CLI compatibility |
| `analyze-skills` | Analyze token usage and dependencies |

When the `subagents` feature is enabled, these additional tools are available:

| Tool | Description |
|------|-------------|
| `list-subagents` | List available subagent specifications |
| `run-subagent` | Execute a subagent with configurable backend |
| `get-run-status` | Check status of a running subagent |
