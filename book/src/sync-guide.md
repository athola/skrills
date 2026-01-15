# Sync Guide

Skrills synchronizes skills, commands, MCP server configurations, and preferences between Claude Code and Codex CLI. This enables you to maintain a single source of truth while using both CLIs.

## Sync Direction

By default, sync operations copy from Claude Code to Codex CLI. Use `--from codex` to reverse the direction.

```bash
# Claude → Codex (default)
skrills sync-all

# Codex → Claude
skrills sync-all --from codex
```

## What Gets Synced

### Skills

Skills are copied between skill directories:
- **Claude**: `~/.claude/skills/`
- **Codex**: `~/.codex/skills/` (discovery root; skills must be `**/SKILL.md`)

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

```bash
skrills sync-commands --from claude
skrills sync-commands --skip-existing-commands  # Don't overwrite existing
```

### MCP Server Configurations

MCP server definitions are synchronized between configuration files:

```bash
skrills sync-mcp-servers --from claude
```

### User Preferences

Preferences and settings are synchronized:

```bash
skrills sync-preferences --from claude
```

## Sync-All Command

The most common workflow is to sync everything at once:

```bash
skrills sync-all --from claude --skip-existing-commands
```

Options:
- `--from`: Source side (`claude` or `codex`)
- `--dry-run`: Preview changes without writing
- `--skip-existing-commands`: Preserve local commands
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

The `mirror` command is a comprehensive sync that also updates `AGENTS.md`:

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
| `sync-from-claude` | Copy Claude skills to Codex (`~/.codex/skills`) |
| `sync-skills` | Sync skills with direction option |
| `sync-commands` | Sync slash commands |
| `sync-mcp-servers` | Sync MCP configurations |
| `sync-preferences` | Sync preferences |
| `sync-all` | Sync everything |
| `sync-status` | Preview sync changes |

## Environment Variables

- `SKRILLS_MIRROR_SOURCE`: Override mirror source root (default `~/.claude`)

## Best Practices

Always preview changes with a dry run before syncing to avoid unexpected overwrites. Sync regularly to keep configurations aligned and prevent drift. After syncing, run `skrills validate` to catch any compatibility issues immediately. To protect your local customizations, use the `--skip-existing-commands` flag. Finally, choose a primary CLI environment and consistently sync from it to maintain a clear source of truth.
