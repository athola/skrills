---
description: Enable skill tracing to track which skills are loaded and invoked.
argument-hint: "[--target claude|codex|both] [--dry-run]"
triggers: enable skill trace, enable tracing, start tracing, turn on tracing
---

# Skill Trace Enable

Enable skill tracing using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__enable-skill-trace` tool with:
- `target`: claude, codex, or both (default: both)
- `instrument`: Add markers to skill files (default: true)
- `backup`: Create .bak files before modifying (default: true)
- `dry_run`: Preview without changes

Parse `$ARGUMENTS` for:
- `--target <target>` or `-t <target>`: Target CLI (default: both)
- `--dry-run` or `-n`: Preview mode
- `--no-backup`: Skip backup when enabling

Report:
- Skills instrumented
- Backup files created (if applicable)
- How to view traces after enabling

Handle errors:
- If target invalid: List valid options (claude, codex, both)
- If skills already instrumented: Report current state, skip re-instrumentation
- If backup fails: Abort and report which file failed
