---
description: Validate skills for Claude Code, Codex, and/or Copilot CLI compatibility.
argument-hint: "[--target claude|codex|copilot|all] [--autofix] [--errors-only]"
triggers: validate skills, check skills, skill errors, skill warnings, skill compatibility
---

# Validate Skills

Validate installed skills using the skrills MCP server.

Use the `mcp__plugin_skrills_skrills__validate-skills` tool with:
- `target`: Validation target (claude, codex, copilot, both, or all). Default: both
- `autofix`: Automatically fix issues when possible
- `errors_only`: Only show skills with errors
- `check_dependencies`: Validate skill dependencies exist

Parse `$ARGUMENTS` for:
- `--target <target>` or `-t <target>`: Which CLI(s) to validate against
- `--autofix` or `-a`: Enable automatic fixes
- `--errors-only` or `-e`: Filter to errors only
- `--deps` or `-d`: Check dependencies

Report validation results including:
- Total skills validated
- Skills with errors (must fix)
- Skills with warnings (should fix)
- Valid skills
- Specific issues and how to fix them

Handle errors:
- If target invalid: List valid options (claude, codex, copilot, both, all)
- If no skills found: Report empty state with setup suggestions
- If autofix fails: Report which fixes failed and why
