---
description: Check skill loading status and instrumentation across CLI environments.
argument-hint: "[--target claude|codex|both]"
triggers: skill trace status, tracing status, is tracing enabled, skill loading status
---

# Skill Trace Status

Check skill tracing status using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__skill-loading-status` tool with:
- `target`: claude, codex, or both (default: both)
- `include_mirror`: Include ~/.codex/skills-mirror
- `include_agent`: Include ~/.agent/skills

Parse `$ARGUMENTS` for:
- `--target <target>` or `-t <target>`: Target CLI (default: both)

Report:
- Tracing enabled/disabled per target
- Instrumented skill count
- Last trace timestamp if available

Handle errors:
- If target invalid: List valid options (claude, codex, both)
- If no skills found: Report empty state with setup suggestions
