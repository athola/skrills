---
description: Fuzzy search installed skills by name or description.
argument-hint: "<query> [--threshold 0.3] [--limit 10]"
triggers: search skills, find skill, lookup skill, skill search, which skill
---

# Search Skills

Search installed skills using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__search-skills-fuzzy` tool with:
- `query`: Search query (required)
- `threshold`: Similarity threshold 0.0-1.0 (lower = more results)
- `limit`: Maximum results to return
- `include_description`: Also search descriptions (default: true)

Parse `$ARGUMENTS`:
- First argument or quoted string: search query
- `--threshold N` or `-t N`: Similarity threshold (default: 0.3)
- `--limit N` or `-l N`: Max results (default: 10)
- `--name-only`: Only search skill names, not descriptions

Report search results including:
- Skill name and location
- Match score
- Brief description
- Source (Claude, Codex, Copilot)

Handle errors:
- If query empty: Prompt for search query
- If no matches found: Suggest broader search or lower threshold
- If index unavailable: Report and suggest re-indexing
