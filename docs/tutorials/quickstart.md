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

First, see what skills you have in your Claude Code skills directory:

```bash
ls ~/.claude/skills
```

Skills can also be discovered from `~/.claude/plugins/cache/` if you have Claude Code plugins installed.

### 2. Validate Skills

Validate skills for compatibility:

```bash
skrills validate
```

This checks that skills work on both platforms:
- **Claude Code**: Permissive validation (frontmatter optional)
- **Codex**: Strict validation (requires `name:` and `description:` frontmatter)

Use `--errors-only` to show only skills with validation issues.

### 3. Analyze Token Usage

Analyze skills for optimization opportunities:

```bash
skrills analyze --suggestions
```

This shows:
- Token count per skill
- Size distribution (small, medium, large)
- Quality scores
- Suggestions for reducing context bloat

Use `--min-tokens` to filter for skills that need optimization attention.

### 4. View Metrics

Get aggregate statistics about your skills:

```bash
skrills metrics
```

Shows:
- Total number of skills
- Skills by source (codex, claude, cache, etc.)
- Quality distribution
- Token statistics
- Dependency information

### 5. Check Sync Status

See what's different between Claude Code and Codex:

```bash
skrills sync-status
```

Displays:
- Skills pending sync
- Configuration differences (MCP servers, preferences)
- Slash commands that need syncing

### 6. Sync Skills to Codex

Copy skills from Claude Code to Codex:

```bash
skrills sync
```

The sync command:
- Converts frontmatter to Codex-compatible format
- Preserves skill content and metadata
- Creates the `~/.codex/skills/` directory if needed
- Enables experimental skills feature in Codex config

After syncing, verify with:
```bash
find ~/.codex/skills -name 'SKILL.md'
```

## Tips

- Use `--dry-run` with any sync command to preview changes
- Run `skrills validate --autofix --backup` to automatically fix issues
- Use `skrills tui` for interactive sync management
- Check `skrills doctor` to diagnose MCP configuration issues
- Set `SKRILLS_INCLUDE_CLAUDE=1` to include Claude plugin cache in discovery

## Requirements

- Rust toolchain (for building from source)
- Claude Code and/or Codex CLI installed
- Skills in `~/.claude/skills/`, `~/.claude/plugins/cache/`, or `~/.codex/skills/`
