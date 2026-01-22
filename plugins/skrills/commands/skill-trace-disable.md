---
description: Disable skill tracing and remove instrumentation markers.
argument-hint: "[--target claude|codex|both] [--dry-run]"
triggers: disable skill trace, disable tracing, stop tracing, turn off tracing
---

# Skill Trace Disable

Disable skill tracing using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__disable-skill-trace` tool with:
- `target`: claude, codex, or both (default: both)
- `dry_run`: Preview without changes

Parse `$ARGUMENTS` for:
- `--target <target>` or `-t <target>`: Target CLI (default: both)
- `--dry-run` or `-n`: Preview mode

Report:
- Skills de-instrumented
- Backup files removed (if any)
- Confirmation tracing is disabled

Handle errors:
- If target invalid: List valid options (claude, codex, both)
- If tracing not enabled: Report current state, no action needed
