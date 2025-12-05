# MCP Token Optimization

## Problem

Interacting with the Machine-Readable Context Protocol (MCP) can be token-intensive, especially when using tools like `list-skills` in clients such as Claude Code. The `list-skills` tool returns full JSON metadata for all discovered skills, which consumes approximately 14,000 tokens and can rapidly exhaust the context window.

## Token-Efficient Solutions

### 1. Use the `render-preview` MCP Tool (~13 tokens)

The `render-preview` MCP tool is the most efficient method for checking skill availability. It provides a minimal JSON response that includes the matched skill count, estimated sizes, and truncation indicators, for a given prompt.

```typescript
mcp__skrills__render-preview
```

### 2. Use the CLI with Filtered Output (~500 tokens)

For a more token-efficient approach, you can extract only skill names using the command-line interface (CLI) with `jq`.

```bash
# List unique skill names
skrills list | jq -r '.[].name' | awk -F'/' '{print $(NF-1)}' | sort -u

# List with source information
skrills list | jq -r '.[] | "\(.name) (\(.source))"'

# Count total skills
skrills list | jq length
```

This reduces token usage by ~95% compared to the `list-skills` tool.

### 3. Use the `list-pinned` Command

To review only your pinned skills, use the `list-pinned` command. This command returns a small subset of skills.

### 4. Use the `history` Command

To see which skills have been automatically loaded in response to recent prompts, use the `history` command. This allows you to understand which skills were used without needing to load their full metadata.

## Comparison

| Method | Token Usage | Use Case |
|---|---|---|
| `list-skills` MCP tool | ~14,000 | Use when full metadata is needed (infrequent). |
| `render-preview` MCP tool| ~13 | For a quick check of skill count and size. |
| CLI with `jq` | ~500 | Provides a human-readable list of skill names. |
| `list-pinned` | < 100 | Specifically for checking only pinned skills. |
| `history` | < 500 | For reviewing recent autoloading activity. |

### Best Practices

### Interactive Sessions
- For interactive exploration in your terminal, use CLI commands.
- Use the `render-preview` tool to estimate the size of autoloaded content.
- Reserve the `list-skills` tool for when you need full skill metadata.

### Automation
- To minimize repetitive token usage, cache the output of `list-skills` locally.
- Use the `refresh-cache` MCP tool to invalidate the local cache when skills are updated.
- Query the local cache instead of making repeated calls to `list-skills`.

## Efficient Skill Discovery Workflow

```bash
# 1. Get a quick count of skills.
skrills list | jq length

# 2. See unique skill names.
skrills list | jq -r '.[].name' | awk -F'/' '{print $(NF-1)}' | sort -u

# 3. Check what has been autoloaded.
skrills history

# 4. If you need full details, read the skill file directly.
cat ~/.codex/skills-mirror/plugins/cache/superpowers/skills/brainstorming/SKILL.md
```

## Available Skills

As of the current version, the `skrills` system discovers 72 unique skills. To retrieve the complete list, execute the following command:
```bash
skrills list | jq -r '.[].name' | awk -F'/' '{print $(NF-1)}' | sort -u
```

## MCP Server Tool Reference

| Tool Name | Token Impact | Description |
|---|---|---|
| `list-skills` | **High (~14k)** | Retrieves comprehensive skill metadata, including paths, hashes, and sources. |
| `autoload-snippet`| Variable | Generates skill content for prompt injection. |
| `render-preview` | **Low (~13)** | Provides a count of matched skills along with size estimates. |
| `runtime-status` | Low (~100) | Shows an overview of the current runtime configuration. |
| `set-runtime-options`| Low | Adjusts manifest and logging settings. |
| `sync-from-claude`| Low | Mirrors skills from Claude. |
| `refresh-cache` | Low | Forces a refresh of the skill cache. |

## See Also

- [CLI Reference](cli.md)
- [Autoload Process](autoload.md)
- [Development Guide](development.md)
