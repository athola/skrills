---
description: Check skill loading status and instrumentation across CLI environments.
argument-hint: "[--target claude|codex|both] [--enable|--disable]"
---

# Skill Trace

Manage skill tracing using the skrills MCP server.

Based on `$ARGUMENTS`:

**Status check (default)**: Use `mcp__plugin_skrills_skrills__skill-loading-status`
- `target`: claude, codex, or both
- `include_mirror`: Include ~/.codex/skills-mirror
- `include_agent`: Include ~/.agent/skills

**Enable tracing** (`--enable`): Use `mcp__plugin_skrills_skrills__enable-skill-trace`
- `target`: claude, codex, or both
- `instrument`: Add markers to skill files
- `backup`: Create .bak files before modifying
- `dry_run`: Preview without changes

**Disable tracing** (`--disable`): Use `mcp__plugin_skrills_skrills__disable-skill-trace`
- `target`: claude, codex, or both
- `dry_run`: Preview without changes

Parse arguments:
- `--target <target>` or `-t <target>`: Target CLI (default: both)
- `--enable` or `-e`: Enable skill tracing
- `--disable` or `-d`: Disable skill tracing
- `--dry-run` or `-n`: Preview mode
- `--no-backup`: Skip backup when enabling

Report tracing status or operation results.
