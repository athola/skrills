---
description: Sync skills and instructions from Claude or Codex to GitHub Copilot CLI.
argument-hint: "[--from claude|codex] [--dry-run]"
---

# Sync to Copilot

Sync to GitHub Copilot CLI using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__sync-to-copilot` tool with:
- `from`: Source agent (claude or codex). Default: claude
- `dry_run`: Preview changes without writing
- `force`: Skip confirmation prompts

Parse `$ARGUMENTS` for:
- `--from <agent>` or `-f <agent>`: Source agent (default: claude)
- `--dry-run` or `-n`: Preview mode
- `--force`: Skip confirmations

This syncs your skills to `~/.config/github-copilot/instructions/`.

Report:
- Skills synced successfully
- Any format conversions applied
- Files created/updated in Copilot directory
