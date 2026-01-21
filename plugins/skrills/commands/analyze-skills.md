---
description: Analyze skills for token usage, dependencies, and optimization suggestions.
argument-hint: "[--min-tokens N] [--no-suggestions]"
---

# Analyze Skills

Analyze installed skills using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__analyze-skills` tool with:
- `min_tokens`: Only include skills with at least this many tokens
- `include_suggestions`: Include optimization suggestions (default: true)

Parse `$ARGUMENTS` for:
- `--min-tokens N` or `-m N`: Filter to skills with N+ tokens
- `--no-suggestions`: Disable optimization suggestions

Report analysis including:
- Total skills and aggregate token usage
- Largest skills by token count
- Quality score distribution
- Dependency patterns
- Optimization recommendations for large/complex skills
