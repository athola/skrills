---
description: Compare a skill across Claude, Codex, and Copilot to show differences.
argument-hint: "<skill-name> [--context <lines>]"
triggers: skill diff, compare skill, skill differences, skill versions, skill comparison
---

# Skill Diff

Compare skill versions across CLIs using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__skill-diff` tool with:
- `name`: Skill name to compare (e.g., "commit", "review-pr")
- `context_lines`: Number of context lines around differences (default: 3)

Parse `$ARGUMENTS` for:
- First positional argument: Skill name (required)
- `--context <n>` or `-C <n>`: Context lines to show

Report comparison results including:
- Which CLIs have the skill
- Unified diff between versions
- Frontmatter differences
- Token count differences
- Whether versions are identical

Handle errors:
- If skill name missing: Prompt for skill name
- If skill not found in any CLI: Report and suggest search command
- If skill exists in only one CLI: Report single-source state
