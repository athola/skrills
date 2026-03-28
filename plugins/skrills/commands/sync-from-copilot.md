---
description: Sync skills and instructions from GitHub Copilot CLI to Claude or Codex.
argument-hint: "[--to claude|codex] [--dry-run]"
triggers: copilot import, import from copilot, copilot to claude, copilot migration
---

# Sync from Copilot

Sync from GitHub Copilot CLI using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__sync-from-copilot` tool with:
- `to`: Target agent (claude or codex). Default: claude
- `dry_run`: Preview changes without writing

Parse `$ARGUMENTS` for:
- `--to <agent>` or `-t <agent>`: Target agent (default: claude)
- `--dry-run` or `-n`: Preview mode

This reads from `~/.config/github-copilot/`:
- Skills and instructions from `instructions/` directory

Report:
- Artifacts synced per type
- Files created/updated in target directory

Handle errors:
- If Copilot directory doesn't exist: Report and exit cleanly
- If target invalid: List valid options (claude, codex)
