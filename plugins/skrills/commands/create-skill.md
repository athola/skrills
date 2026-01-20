---
description: Create a new skill via GitHub search, LLM generation, or both.
argument-hint: "<name> <description> [--method github|llm|both] [--dry-run]"
---

# Create Skill

Create a new skill using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__create-skill` tool with:
- `name`: Name or topic for the skill (required)
- `description`: What the skill should do (required)
- `method`: Creation method - github (search), llm (generate), both, or empirical
- `target_dir`: Directory to create skill in
- `dry_run`: Preview without creating files

Parse `$ARGUMENTS`:
- First argument: skill name
- Remaining text: skill description
- `--method <method>` or `-m <method>`: Creation method (default: both)
- `--dry-run` or `-n`: Preview mode
- `--dir <path>`: Target directory

Default behavior:
1. Search GitHub for existing skills matching the name/description
2. If found, offer to download and adapt
3. If not found or declined, generate a new skill using LLM

Report:
- Search results (if method includes github)
- Generated skill content (if method includes llm)
- File location where skill was created
