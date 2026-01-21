---
description: Preview what would be synced between AI coding assistants without making changes.
argument-hint: "[--from claude|codex|copilot] [--to claude|codex|copilot]"
---

# Sync Status

Show sync status and preview changes using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__sync-status` tool with:
- `from`: Source agent (claude, codex, or copilot)
- `to`: Target agent
- `dry_run`: Always true for status checks

Parse `$ARGUMENTS` for:
- `--from <agent>` or `-f <agent>`: Source agent (default: claude)
- `--to <agent>` or `-t <agent>`: Target agent (default: codex)

Report:
- Skills that would be synced (new, updated, unchanged)
- Commands that would be synced
- MCP server configs that would be synced
- Any potential conflicts
