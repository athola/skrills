# CLI Usage Reference

Reference for `skrills` CLI subcommands and usage.

## `serve`

Starts the MCP server over stdio.

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

## `metrics`

Shows aggregate statistics about discovered skills including counts, quality distribution, dependency patterns, and token usage.

```bash
skrills metrics [OPTIONS]
```

Options:
- `--skill-dir <DIR>`: Skills directory to include (default: all discovered skills)
- `--format <FORMAT>`: Output format: `text` or `json` (default: `text`)
- `--include-validation`: Include validation summary (slower)

Examples:
```bash
skrills metrics                              # Human-readable summary
skrills metrics --format json                # Machine-readable output
skrills metrics --include-validation         # Include pass/fail counts
```

Output includes:
- Total skill count by source (claude, codex, marketplace)
- Quality distribution (high/medium/low based on quality scores)
- Dependency statistics (total edges, orphan count, hub skills)
- Token usage (total, average, largest skill)

## `sync`

Copies skills from `~/.claude` into `~/.codex/skills` (Codex discovery root).

```bash
skrills sync
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

### Skill naming caveat

Skills are named from the `name:` field in `SKILL.md` frontmatter. Treat these names as opaque strings: they
may include punctuation such as `:` for namespacing (for example, `pensive:shared`).

If you’re diffing “skills listed in a session header” vs what exists on disk, don’t parse by splitting on
`:`. Prefer extracting the `(file: …/SKILL.md)` path, or read the `SKILL.md` frontmatter directly.

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

When no backend is specified in an agent spec, skrills checks `~/.codex/subagents.toml` for a `default_backend`; if absent it uses `SKRILLS_SUBAGENTS_DEFAULT_BACKEND`, defaulting to `codex`.

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

`skrills` exposes these tools via the MCP protocol:

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
| `skill-metrics` | Aggregate statistics (quality, tokens, dependencies) |
| `skill-loading-status` | Report skill roots, trace/probe install status, and marker coverage |
| `enable-skill-trace` | Install trace/probe skills and optionally instrument SKILL.md files with markers |
| `disable-skill-trace` | Remove trace/probe skill directories (does not remove markers) |
| `skill-loading-selftest` | Return a one-shot probe line and expected response to confirm skills are loading |

When the `subagents` feature is enabled, these additional tools are available:

| Tool | Description |
|------|-------------|
| `list-subagents` | List available subagent specifications |
| `run-subagent` | Execute a subagent with configurable backend |
| `get-run-status` | Check status of a running subagent |

## Skill loading validation

Use the trace/probe tools when you need a deterministic signal that skills are loading in the current Claude Code or Codex session.

Workflow:

1. Call `enable-skill-trace` (use `dry_run: true` to preview). This installs two debug skills and can instrument skill files by appending `<!-- skrills-skill-id: ... -->` markers (with optional backups).
2. Restart the session if the client does not hot-reload skills.
3. Call `skill-loading-selftest` and send the returned `probe_line`. Expect `SKRILLS_PROBE_OK:<token>`.
4. With tracing enabled and markers present, each assistant response should end with `SKRILLS_SKILLS_LOADED: [...]` and `SKRILLS_SKILLS_USED: [...]`.

Use `skill-loading-status` to confirm which roots were scanned and whether markers are present. Use `disable-skill-trace` to remove the debug skills when finished.
