---
description: Identify gaps in your skill library and suggest new skills to create based on project context.
argument-hint: "[--focus <area>] [--project-dir <path>]"
triggers: suggest skills, skill gaps, missing skills, what skills, recommend skills
---

# Suggest Skills

Get suggestions for new skills to create using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__suggest-new-skills` tool with:
- `project_dir`: Project directory for context analysis
- `focus_areas`: Specific areas to focus on (e.g., testing, deployment)

Parse `$ARGUMENTS` for:
- `--focus <area>` or `-f <area>`: Focus on specific area (can be repeated)
- `--project-dir <path>` or `-p <path>`: Project to analyze

Analyze the current project context and identify:
- Missing skills based on project technologies
- Gaps in workflow coverage
- Skills that similar projects typically have

Report suggestions including:
- Suggested skill name and purpose
- Why this skill would be useful
- Estimated complexity
- Similar existing skills that could be adapted

Handle errors:
- If project directory invalid: Use current directory or prompt for path
- If no context available: Provide generic suggestions based on focus area
