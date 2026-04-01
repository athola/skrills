# Sync Guide

Skrills synchronizes skills, commands, rules, MCP server configurations, and preferences between Claude Code, Codex CLI, GitHub Copilot CLI, and Cursor IDE. This enables you to maintain a single source of truth while using multiple CLIs.

## Sync Direction

By default, `sync-all` syncs from Claude to **all other CLIs** (Codex, Copilot, and Cursor). No flags required.

```bash
# Claude → ALL other CLIs (simplest form)
skrills sync-all

# Codex → ALL other CLIs
skrills sync-all --from codex

# Claude → Codex only (specific target)
skrills sync-all --to codex

# Claude → Cursor only
skrills sync-all --to cursor

# Cursor → Claude
skrills sync-all --from cursor --to claude
```

## What Gets Synced

### Skills

Skills are copied between skill directories:
- **Claude**: `~/.claude/skills/`
- **Codex**: `~/.codex/skills/` (discovery root; skills must be `**/SKILL.md`)
- **Copilot**: `~/.copilot/skills/` (same SKILL.md format as Codex)
- **Cursor**: `~/.cursor/skills/{name}/SKILL.md` (frontmatter stripped on write)

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
- **Cursor**: `~/.cursor/commands/{name}.md`

```bash
skrills sync-commands --from claude --to codex
skrills sync-commands --from claude --to codex --skip-existing-commands
```

### MCP Server Configurations

MCP server definitions are synchronized between configuration files:
- **Claude/Codex**: Share similar MCP configuration format
- **Copilot**: Uses `mcp-config.json` (separate from main config)
- **Cursor**: Uses `.cursor/mcp.json`

Tool filtering fields (`allowedTools`/`disabledTools`) are preserved during sync. When set, `allowed_tools` restricts which tools are available from a server and `disabled_tools` hides specific tools from the model.

```bash
skrills sync-mcp-servers --from claude --to codex
skrills sync-mcp-servers --from claude --to copilot
skrills sync-mcp-servers --from claude --to cursor
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

### Shorthand Sync Commands

For convenience, shorthand commands sync from a specific CLI to all others:

```bash
skrills sync-from-claude     # Claude as source of truth
skrills sync-from-codex      # Codex as source of truth
skrills sync-from-copilot    # Copilot as source of truth
skrills sync-from-cursor     # Cursor as source of truth (converts .mdc back to SKILL.md)
```

These are equivalent to `skrills sync-all --from <cli>`.

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

### Cursor Rules

Cursor IDE stores project instructions as `.mdc` files in `.cursor/rules/` with YAML frontmatter controlling when each rule applies. Skrills maps between Cursor rules and Claude/Copilot instruction formats:

| Source | Cursor Rule Mode | Frontmatter |
|--------|-----------------|-------------|
| `CLAUDE.md` or `claude-instructions` | Always-apply | `alwaysApply: true` |
| Source with `globs:` in frontmatter | Auto-attach | Preserve globs, `alwaysApply: false` |
| Other instructions | Agent-requested | `description: "Rule: {name}"`, `alwaysApply: false` |

```bash
# Sync rules from Cursor to Claude
skrills sync-all --from cursor --to claude

# Sync Claude instructions to Cursor rules
skrills sync-all --from claude --to cursor
```

Cursor-only frontmatter fields (`globs`, `alwaysApply`) are preserved in content bytes during Cursor-to-Claude sync but have no effect in Claude. See [ADR 0006](../docs/adr/0006-cursor-rules-mapping.md) for the mapping rationale.

### Cursor Hooks

Hook event names are translated between Claude's PascalCase and Cursor's camelCase conventions:

| Claude (PascalCase) | Cursor (camelCase) |
|---------------------|--------------------|
| `PreToolUse` | `preToolUse` |
| `PostToolUse` | `postToolUse` |
| `SessionStart` | `sessionStart` |
| `SessionEnd` | `sessionEnd` |
| `Stop` | `stop` |
| `SubagentStop` | `subagentStop` |
| `UserPromptSubmit` | `beforeSubmitPrompt` |
| `PreCompact` | `preCompact` |

The `Notification` event has no Cursor equivalent and is skipped during sync. See [ADR 0006](../docs/adr/0006-cursor-rules-mapping.md) for the full mapping rationale.

Cursor-only events (`afterFileEdit`, `beforeShellExecution`) are preserved in raw content during Cursor-to-Claude sync.

### Cursor Agents

Agent fields are translated:
- `background` (Claude) ↔ `is_background` (Cursor)

## MCP Tools

When running as an MCP server, these sync tools are available:

| Tool | Description |
|------|-------------|
| `sync-from-claude` | Copy Claude skills to Codex, Copilot, or Cursor |
| `sync-from-copilot` | Copy Copilot skills to Claude, Codex, or Cursor |
| `sync-from-cursor` | Copy Cursor skills/rules to Claude, Codex, or Copilot |
| `sync-to-copilot` | Copy skills from Claude, Codex, or Cursor to Copilot |
| `sync-to-cursor` | Copy skills/rules from Claude, Codex, or Copilot to Cursor |
| `sync-skills` | Sync skills with direction option |
| `sync-commands` | Sync slash commands (Claude/Codex/Cursor) |
| `sync-mcp-servers` | Sync MCP configurations |
| `sync-preferences` | Sync preferences |
| `sync-all` | Sync everything |
| `sync-status` | Preview sync changes |

## Environment Variables

- `SKRILLS_MIRROR_SOURCE`: Override mirror source root (default `~/.claude`)

## Best Practices

Always preview changes with a dry run before syncing to avoid unexpected overwrites. Sync regularly to keep configurations aligned and prevent drift. After syncing, run `skrills validate` to catch any compatibility issues immediately. To protect your local customizations, use the `--skip-existing-commands` flag. Finally, choose a primary CLI environment and consistently sync from it to maintain a clear source of truth.
