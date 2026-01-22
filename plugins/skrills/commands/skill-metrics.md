---
description: Get aggregate statistics about discovered skills including counts, quality, and token usage.
argument-hint: "[--include-validation]"
triggers: skill metrics, skill stats, skill statistics, how many skills, skill count
---

# Skill Metrics

Get skill metrics and statistics using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__skill-metrics` tool with:
- `include_validation`: Include validation summary (slower but more complete)

Parse `$ARGUMENTS` for:
- `--include-validation` or `-v`: Include validation stats

Report metrics including:
- Total skill count by source (Claude, Codex, Copilot)
- Quality score distribution (excellent, good, fair, poor)
- Token usage statistics (total, average, max)
- Dependency patterns (most depended-on skills)
- Common tags and categories

Handle errors:
- If no skills found: Report empty state with setup suggestions
- If validation fails: Report partial metrics with warning
