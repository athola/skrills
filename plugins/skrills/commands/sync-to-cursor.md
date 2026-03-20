---
description: Sync skills, commands, agents, hooks, rules, and MCP from Claude, Codex, or Copilot to Cursor IDE.
argument-hint: "[--from claude|codex|copilot] [--dry-run]"
triggers: cursor sync, export to cursor, cursor rules, cursor skills, migrate to cursor
---

# Sync to Cursor

Sync to Cursor IDE using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__sync-to-cursor` tool with:
- `from`: Source agent (claude, codex, or copilot). Default: claude
- `dry_run`: Preview changes without writing

Parse `$ARGUMENTS` for:
- `--from <agent>` or `-f <agent>`: Source agent (default: claude)
- `--dry-run` or `-n`: Preview mode

This syncs to `~/.cursor/`:
- Skills → `.cursor/skills/{name}/SKILL.md` (frontmatter stripped)
- Commands → `.cursor/commands/{name}.md`
- Agents → `.cursor/agents/{name}.md` (field translation: background→is_background)
- Hooks → `.cursor/hooks.json` (event names: PascalCase→camelCase)
- Rules → `.cursor/rules/{name}.mdc` (CLAUDE.md→alwaysApply rule)
- MCP → `.cursor/mcp.json`

Report:
- Artifacts synced per type (skills, commands, agents, hooks, rules, MCP)
- Skipped items with reasons (e.g., Notification hook has no Cursor equivalent)
- Files created/updated in Cursor directory

Handle errors:
- If Cursor directory doesn't exist: Offer to create it
- If source invalid: List valid options (claude, codex, copilot)
- If write permission denied: Report path and suggest fix
