---
description: Sync skills, commands, MCP servers, and preferences between Claude, Codex, and Copilot in one operation.
argument-hint: "[--from claude|codex|copilot] [--to claude|codex|copilot] [--dry-run]"
---

# Sync All

Sync all configurations between AI coding assistants using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__sync-all` tool with these parameters:
- `from`: Source agent (claude, codex, or copilot). Default: claude
- `to`: Target agent. Default: codex (for claude source) or claude (for others)
- `dry_run`: Set to true to preview changes without writing
- `force`: Set to true to skip confirmation prompts

Parse `$ARGUMENTS` for:
- `--from <agent>` or `-f <agent>`: Source agent
- `--to <agent>` or `-t <agent>`: Target agent
- `--dry-run` or `-n`: Preview mode
- `--force`: Skip confirmations

If no arguments provided, sync from Claude to Codex with a preview first.

Report sync results including:
- Number of skills synced
- Number of commands synced
- Number of MCP servers synced
- Any conflicts or errors encountered
