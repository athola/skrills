---
description: Sync skills, commands, agents, hooks, rules, and MCP from Cursor IDE to Claude, Codex, or Copilot.
argument-hint: "[--to claude|codex|copilot] [--dry-run]"
triggers: cursor import, import from cursor, cursor to claude, cursor migration
---

# Sync from Cursor

Sync from Cursor IDE using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__sync-from-cursor` tool with:
- `to`: Target agent (claude, codex, or copilot). Default: claude
- `dry_run`: Preview changes without writing

Parse `$ARGUMENTS` for:
- `--to <agent>` or `-t <agent>`: Target agent (default: claude)
- `--dry-run` or `-n`: Preview mode

This reads from `~/.cursor/`:
- Skills from `.cursor/skills/{name}/SKILL.md`
- Commands from `.cursor/commands/{name}.md`
- Agents from `.cursor/agents/{name}.md` (field translation: is_background→background)
- Hooks from `.cursor/hooks.json` (event names: camelCase→PascalCase)
- Rules from `.cursor/rules/*.mdc` (alwaysApply→CLAUDE.md, globs preserved)
- MCP from `.cursor/mcp.json`

Report:
- Artifacts synced per type
- Cursor-only hook events (afterFileEdit, beforeShellExecution) preserved in raw content
- Files created/updated in target directory

Handle errors:
- If Cursor directory doesn't exist: Report and exit cleanly
- If target invalid: List valid options (claude, codex, copilot)
