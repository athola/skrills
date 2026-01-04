# Quickstart: Unified Skills for Claude Code & Codex

This tutorial shows how to use Skrills to validate, analyze, and synchronize your skills between Claude Code and Codex CLI.

![Quickstart Demo](../../assets/gifs/quickstart.gif)

## Overview

Skrills provides bidirectional synchronization with validation and analysis for skills that work on both Claude Code and Codex.

**Key Commands:**
- `skrills validate` - Check skills for compatibility issues
- `skrills analyze` - Review token usage and optimization opportunities
- `skrills sync-status` - See differences between platforms
- `skrills sync` - Copy skills from Claude Code to Codex
- `skrills metrics` - View aggregate statistics

## Step-by-Step Walkthrough

### 1. Check Available Skills

First, see how many skills you have available:

```bash
find ~/.claude/plugins/cache -name 'SKILL.md' | wc -l
```

This counts all SKILL.md files in your Claude plugin cache. In the demo, we have 400+ skills available.

### 2. Validate Skills

Validate a sample of skills for compatibility:

```bash
skrills validate --sample 5 --errors-only
```

This checks that skills work on both platforms:
- **Claude Code**: Permissive validation (frontmatter optional)
- **Codex**: Strict validation (requires `name:` and `description:` frontmatter)

The `--sample 5` flag validates 5 random skills for quick testing. Use without `--sample` to validate all skills.

### 3. Analyze Token Usage

Analyze skills for optimization opportunities:

```bash
skrills analyze --sample 10 --suggestions --min-tokens 300
```

This shows:
- Token count per skill
- Dependencies between skills
- Suggestions for reducing context bloat
- Skills exceeding the token threshold

Use `--min-tokens` to filter for skills that need optimization attention.

### 4. Check Sync Status

See what's different between Claude Code and Codex:

```bash
skrills sync-status
```

Displays:
- Skills present in one platform but not the other
- Configuration differences (MCP servers, preferences)
- Slash commands that need syncing

### 5. Sync Skills to Codex

Copy skills from Claude Code to Codex:

```bash
skrills sync --sample 5
```

This syncs a sample of 5 skills. For production use, sync all skills:

```bash
skrills sync
```

The sync command:
- Converts frontmatter to Codex-compatible format
- Preserves skill content and metadata
- Creates the `~/.codex/skills/` directory if needed

### 6. View Metrics

Get aggregate statistics about your skills:

```bash
skrills metrics
```

Shows:
- Total number of skills
- Average token count
- Skills by category/type
- Dependency information

## Tips

- Use `--dry-run` with any sync command to preview changes
- Run `skrills validate --autofix --backup` to automatically fix issues
- Use `skrills tui` for interactive sync management
- Check `skrills doctor` to diagnose MCP configuration issues

## Requirements

- Rust toolchain (for building from source)
- Claude Code and/or Codex CLI installed
- Skills in `~/.claude/plugins/cache/` or `~/.codex/skills/`
