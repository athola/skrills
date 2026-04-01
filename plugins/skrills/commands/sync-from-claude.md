---
description: Sync skills from Claude Code (~/.claude) into Codex, Cursor, or Copilot.
argument-hint: "[--to codex|cursor|copilot] [--include-marketplace] [--dry-run]"
triggers: claude import, import from claude, claude to codex, claude to cursor, claude migration
---

# Sync from Claude

Sync from Claude Code using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__sync-from-claude` tool with:
- `to`: Target agent (`codex`, `cursor`, or `copilot`). Default: `codex`
- `include_marketplace`: Include marketplace skills (default: false)
- `dry_run`: Preview changes without writing

Parse `$ARGUMENTS` for:
- `--to <agent>` or `-t <agent>`: Target agent (default: codex)
- `--include-marketplace`: Include marketplace skills
- `--dry-run` or `-n`: Preview mode

This reads from `~/.claude/`:
- Skills from SKILL.md files

And syncs to the target directory:
- Codex: `~/.codex/skills/` (Codex discovery root)
- Cursor: `.cursor/` directory (agents, rules, skills)
- Copilot: `~/.config/github-copilot/instructions/`

Report:
- Number of skills copied
- Number of skills skipped
- Names of synced skills

Handle errors:
- If Claude directory doesn't exist: Report and exit cleanly
- If target directory doesn't exist: Offer to create it
