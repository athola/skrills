# MCP Token Optimization

## Overview

The Skrills MCP server minimizes token usage by returning targeted summaries rather than full skill metadata payloads.

## MCP Tools Token Impact

| Tool | Token Impact | Description |
|------|-------------|-------------|
| `validate-skills` | Low-Medium | Returns validation results; scales with skill count |
| `analyze-skills` | Low-Medium | Returns analysis results; scales with skill count |
| `sync-from-claude` | Low | Returns sync summary |
| `sync-skills` | Low | Returns sync summary |
| `sync-commands` | Low | Returns sync summary |
| `sync-mcp-servers` | Low | Returns sync summary |
| `sync-preferences` | Low | Returns sync summary |
| `sync-all` | Low | Returns combined sync summary |
| `sync-status` | Low | Returns diff preview |

When the `subagents` feature is enabled:

| Tool | Token Impact | Description |
|------|-------------|-------------|
| `list-subagents` | Low | Returns list of available subagents |
| `run-subagent` | Variable | Depends on subagent output |
| `get-run-status` | Low | Returns status of running subagent |

## Best Practices

### Prefer CLI for Batch Operations
For operations involving many skills, the CLI is more efficient than repeated MCP tool calls. A single CLI command like `skrills validate --format json` can replace hundreds of individual tool invocations.

### Filter Output
Use filtering options to reduce payload size. For example, `skrills validate --errors-only` returns only the skills that failed validation, and `skrills analyze --min-tokens 2000` limits the output to only the largest skills.

### Preview Before Sync
Use `skrills sync-status --from claude` to preview changes before running a full sync. This shows the scope of changes without the overhead of a full write operation.

## Efficient Workflows

### Validation Workflow

```bash
# 1. Preview validation (errors only)
skrills validate --errors-only

# 2. Fix issues with autofix
skrills validate --target codex --autofix --backup

# 3. Verify fixes
skrills validate --target codex --errors-only
```

### Sync Workflow

```bash
# 1. Preview changes
skrills sync-status --from claude

# 2. Sync if changes look correct
skrills sync-all --from claude --skip-existing-commands
```

## See Also

- [CLI Usage Reference](cli.md)
- [Skill Validation](validation.md)
- [Sync Guide](sync-guide.md)
