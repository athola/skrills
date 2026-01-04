# Quickstart: Unified Skills for Claude Code & Codex

This tutorial shows how to use Skrills to keep your skills synchronized between Claude Code and Codex CLI.

![Quickstart Demo](../../assets/gifs/quickstart.gif)

## The Problem

Both Claude Code and Codex use markdown-based skills (SKILL.md files), but they have different:

- **Discovery locations**: Claude Code uses `~/.claude/commands/`, Codex uses `~/.codex/skills/`
- **Frontmatter requirements**: Codex requires strict YAML frontmatter; Claude Code is more permissive
- **Configuration formats**: MCP servers, preferences, and slash commands are stored differently

Manually keeping these in sync is tedious and error-prone.

## The Solution

Skrills provides bidirectional sync with validation:

```bash
# See what's different
skrills sync-status

# Validate skills work on both platforms
skrills validate --errors-only

# Analyze token usage for optimization
skrills analyze --suggestions

# Sync everything
skrills sync-all
```

## Step-by-Step Walkthrough

### 1. Check Sync Status

```bash
skrills sync-status
```

Shows what's different between Claude Code and Codex:
- Skills present in one but not the other
- Configuration differences (MCP servers, preferences)
- Slash commands that need syncing

### 2. Validate Skills

```bash
skrills validate --errors-only
```

Checks that your skills are compatible with both platforms:
- **Claude Code**: Permissive validation (frontmatter optional)
- **Codex**: Strict validation (requires `name:` and `description:` frontmatter)

Fix issues with `--autofix`:

```bash
skrills validate --autofix --backup
```

### 3. Analyze Token Usage

```bash
skrills analyze --suggestions --min-tokens 500
```

Helps optimize context usage by showing:
- Token count per skill
- Dependencies between skills
- Suggestions for reducing bloat

### 4. Sync Everything

```bash
# Preview what would change
skrills sync-all --dry-run

# Actually sync
skrills sync-all
```

Syncs bidirectionally:
- Skills (`~/.claude/commands/` ↔ `~/.codex/skills/`)
- MCP server configurations
- Preferences
- Slash commands

## Next Steps

- Run `skrills doctor` to diagnose MCP configuration issues
- Use `skrills tui` for interactive sync management
- Set up `skrills serve` as an MCP server for live skill loading

## Requirements

- Rust toolchain (for building from source)
- Claude Code and/or Codex CLI installed
- Skills in `~/.claude/commands/` or `~/.codex/skills/`
