---
description: Sync skills from Claude Code (~/.claude) into Codex (~/.codex/skills).
argument-hint: "[--include-marketplace] [--dry-run]"
triggers: claude import, import from claude, claude to codex, claude migration
---

# Sync from Claude

Sync from Claude Code using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__sync-from-claude` tool with:
- `include_marketplace`: Include marketplace skills (default: false)
- `dry_run`: Preview changes without writing

Parse `$ARGUMENTS` for:
- `--include-marketplace`: Include marketplace skills
- `--dry-run` or `-n`: Preview mode

This reads from `~/.claude/`:
- Skills from SKILL.md files

And syncs to `~/.codex/skills/` (Codex discovery root).

Report:
- Number of skills copied
- Number of skills skipped
- Names of synced skills

Handle errors:
- If Claude directory doesn't exist: Report and exit cleanly
- If Codex skills directory doesn't exist: Offer to create it
