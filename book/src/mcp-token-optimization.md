# MCP Token Optimization

## Problem

Using the `list-skills` tool within an MCP client can lead to high token consumption (approximately 14,000 tokens), because it returns all JSON metadata for all discovered skills. This depletes the available context window.

## Token-Efficient Solutions

### 1. Quick Checks: `render-preview` (~13 tokens)

The `render-preview` tool quickly assesses what content would be loaded without high token costs. It provides a concise JSON response containing matched skill counts, size estimates, and truncation indicators.

```javascript
mcp__skrills__render-preview
```

### 2. Human-Readable Lists: CLI + `jq` (~500 tokens)

To get human-readable lists of skill names, use the CLI with `jq`.

```bash
# Lists unique skill names
skrills list | \
  jq -r '.[].name' | \
  awk -F'/' '{print $(NF-1)}' | sort -u

# Lists skill names with their sources
skrills list | \
  jq -r '.[] | "\(.name) (\(.source))"'

# Counts the total number of skills
skrills list | jq length
```

This approach achieves a 95% token reduction compared to `list-skills`.

### 3. Check Pinned Skills: `list-pinned` (minimal tokens)

Displays only explicitly pinned skills.

```bash
skrills list-pinned
```

### 4. Understand Autoload: `history` (minimal tokens)

Shows a history of recently autoloaded skills.

```bash
skrills history
```

## Comparison

| Method | Token Usage | Use Case |
|---|---|---|
| `list-skills` MCP tool | ~14,000 | Use when full metadata is needed (infrequent). |
| `render-preview` MCP tool| ~13 | For a quick check of skill count and size. |
| CLI with `jq` | ~500 | Provides a human-readable list of skill names. |
| `list-pinned` | < 100 | Specifically for checking only pinned skills. |
| `history` | < 500 | For reviewing recent autoloading activity. |

## Efficient Skill Discovery Workflow

This workflow minimizes token costs during skill discovery.

```bash
# 1. Get a quick count of skills.
skrills list | jq length

# 2. See unique skill names.
skrills list | \
    jq -r ".[].name" | awk -F'/' '{print $(NF-1)}' | sort -u

# 3. Check what has been autoloaded.
skrills history

# 4. If you need full details, read the skill file directly.
cat ~/.codex/skills-mirror/plugins/cache/superpowers/skills/brainstorming/SKILL.md
```

## Available Skills Summary

The `skrills` system discovers 72 unique skills, categorized as follows:

- **Core Development Workflows**: Includes skills for brainstorming, systematic debugging, test-driven development, and verification processes.
- **Code Review & Quality**: Features specialized skills for code review, such as `test-review`, `bug-review`, and `rust-review`.
- **Documentation & Git**: Covers skills related to documentation updates, README file management, versioning, crafting commit messages, and preparing Pull Requests.
- **Architecture Patterns**: Encompasses 14 distinct architectural paradigms, including functional-core, hexagonal, and microservices.
- **Language Ecosystems**: Offers Python-focused skills covering areas like testing, asynchronous programming patterns, packaging, and performance optimization.
- **Infrastructure & Cloud**: Includes skills for multi-cloud architecture, Terraform, hybrid networking, and performance tuning.
- **Testing Philosophy**: Addresses advanced testing concepts, such as identifying testing anti-patterns and implementing defense-in-depth strategies.
- **Meta Skills**: Provides skills specifically designed for working with other skills, such as writing new skills or testing skills with subagents.

To get the complete list, run the `jq` command above.

## MCP Server Tool Reference

The `skrills` MCP server exposes seven tools:

| Tool Name | Token Impact | Description |
|---|---|---|
| `list-skills` | High (~14k) | Retrieves comprehensive skill metadata. |
| `render-preview` | Low (~13) | Provides a count of matched skills along with size estimates. |
| `runtime-status` | Low (~100) | Shows an overview of the current runtime configuration. |
| `autoload-snippet` | Variable | Generates skill content for prompt injection. |
| `set-runtime-options` | Minimal | Adjusts manifest and logging settings. |
| `sync-from-claude` | Minimal | Mirrors skills from Claude. |
| `refresh-cache` | Minimal | Forces a refresh of the skill cache. |

## Best Practices

- **Cache It**: For automation needing full skill metadata, run `list-skills` once, save the output locally, and reuse it. Invalidate the cache with `refresh-cache` when skill definitions change.
- **Read Directly**: Since skills are Markdown files, you can read known skills directly from [`~/.codex/skills-mirror/`](~/.codex/skills-mirror/) to bypass the MCP layer for speed and token efficiency.
- **Monitor Autoload**: Use the `history` command to see which skills are triggered in production. This helps debug unexpected autoloads or fine-tune prompt keywords.

## See Also

- [Autoloading Skills and Context](autoload.md)
- [CLI Usage Reference](cli.md)
- [Development Guide](development.md)