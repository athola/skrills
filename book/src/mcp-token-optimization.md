# MCP Token Optimization

## Overview

When using skrills as an MCP server, token usage is generally low because the server focuses on targeted operations (validation, analysis, sync) rather than returning large skill metadata payloads.

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

### Use CLI for Batch Operations

For operations on many skills, the CLI is often more efficient than repeated MCP tool calls:

```bash
# Validate all skills (single operation)
skrills validate --format json

# Analyze skills exceeding threshold
skrills analyze --min-tokens 1000 --format json
```

### Filter Output

Use output filtering options to reduce payload size:

```bash
# Only show errors
skrills validate --errors-only

# Only show large skills
skrills analyze --min-tokens 2000
```

### Preview Before Sync

Use `sync-status` to preview changes before running a full sync:

```bash
skrills sync-status --from claude
```

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
