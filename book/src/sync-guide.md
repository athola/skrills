# Sync Guide

Skrills synchronizes skills, commands, MCP server configurations, and preferences between Claude Code, Codex CLI, and GitHub Copilot CLI. This enables you to maintain a single source of truth while using multiple CLIs.

## Sync Direction

By default, `sync-all` syncs from Claude to **all other CLIs** (Codex and Copilot). No flags required.

```bash
# Claude → ALL other CLIs (simplest form)
skrills sync-all

# Codex → ALL other CLIs
skrills sync-all --from codex

# Claude → Codex only (specific target)
skrills sync-all --to codex
```

## What Gets Synced

### Skills

Skills are copied between skill directories:
- **Claude**: `~/.claude/skills/`
- **Codex**: `~/.codex/skills/` (discovery root; skills must be `**/SKILL.md`)
- **Copilot**: `~/.copilot/skills/` (same SKILL.md format as Codex)

Codex skills are disabled by default; enable them in `~/.codex/config.toml`:

```toml
[features]
skills = true
```

```bash
skrills sync                # Skills only
skrills sync-all            # Everything including skills
```

### Slash Commands

Commands are synced byte-for-byte, preserving non-UTF-8 content:
- **Claude**: `~/.claude/commands/`
- **Codex**: `~/.codex/prompts/`
- **Copilot**: Does not support slash commands (skipped during sync)

```bash
skrills sync-commands --from claude --to codex
skrills sync-commands --from claude --to codex --skip-existing-commands
```

### MCP Server Configurations

MCP server definitions are synchronized between configuration files:
- **Claude/Codex**: Share similar MCP configuration format
- **Copilot**: Uses `mcp-config.json` (separate from main config)

```bash
skrills sync-mcp-servers --from claude --to codex
skrills sync-mcp-servers --from claude --to copilot
```

### User Preferences

Preferences and settings are synchronized:
- **Copilot**: Uses `config.json` with security field preservation

```bash
skrills sync-preferences --from claude --to codex
skrills sync-preferences --from claude --to copilot
```

## Sync-All Command

The most common workflow is to sync everything at once:

```bash
# Sync from Claude to all other CLIs (no flags needed)
skrills sync-all

# Sync to a specific target only
skrills sync-all --to codex --skip-existing-commands
```

Options:
- `--from`: Source CLI (default: `claude`)
- `--to`: Target CLI (default: all other CLIs)
- `--dry-run`: Preview changes without writing
- `--skip-existing-commands`: Preserve local commands (Claude/Codex only)
- `--validate`: Run validation after sync
- `--autofix`: Auto-fix validation issues

## Preview Changes

Before syncing, preview what will change:

```bash
skrills sync-status --from claude
```

This shows:
- Files that would be added
- Files that would be updated
- Configuration differences

## Mirror Command

The `mirror` command syncs files and updates `AGENTS.md`:

```bash
skrills mirror --skip-existing-commands
```

Use `--dry-run` to preview:

```bash
skrills mirror --dry-run
```

## MCP Tools

When running as an MCP server, these sync tools are available:

| Tool | Description |
|------|-------------|
| `sync-from-claude` | Copy Claude skills to Codex or Copilot |
| `sync-from-copilot` | Copy Copilot skills to Claude or Codex |
| `sync-to-copilot` | Copy skills from Claude or Codex to Copilot |
| `sync-skills` | Sync skills with direction option (all 6 combinations) |
| `sync-commands` | Sync slash commands (Claude/Codex only) |
| `sync-mcp-servers` | Sync MCP configurations |
| `sync-preferences` | Sync preferences |
| `sync-all` | Sync everything |
| `sync-status` | Preview sync changes |

## Environment Variables

- `SKRILLS_MIRROR_SOURCE`: Override mirror source root (default `~/.claude`)

## Best Practices

Always preview changes with a dry run before syncing to avoid unexpected overwrites. Sync regularly to keep configurations aligned and prevent drift. After syncing, run `skrills validate` to catch any compatibility issues immediately. To protect your local customizations, use the `--skip-existing-commands` flag. Finally, choose a primary CLI environment and consistently sync from it to maintain a clear source of truth.
