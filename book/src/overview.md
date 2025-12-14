# Project Overview

`skrills` is a skills support engine for Claude Code and Codex CLI. It validates skills for compatibility, analyzes token usage, and synchronizes configurations bidirectionally between both CLIs.

## Core Capabilities

- **Validation**: Validates skills against Claude Code (permissive) and Codex CLI (strict) requirements. Includes auto-fix capability to add missing frontmatter.
- **Analysis**: Analyzes skills for token usage, dependencies, and optimization opportunities.
- **Bidirectional Sync**: Synchronizes skills, slash commands, MCP server configurations, and preferences between Claude Code and Codex CLI.
- **MCP Server**: Operates over standard I/O (stdio), providing tools for validation, analysis, and sync operations.
- **Subagents Runtime**: Provides MCP tools for executing subagents with configurable backends (Claude-style or Codex-style).

## Architecture

Skrills is organized as a Rust workspace with specialized crates:

- `crates/server`: MCP server runtime and CLI.
- `crates/validate`: Skill validation for Claude Code and Codex CLI compatibility.
- `crates/analyze`: Token counting, dependency analysis, and optimization suggestions.
- `crates/sync`: Bidirectional sync between Claude/Codex (skills, commands, prefs, MCP servers).
- `crates/discovery`: Skill discovery and ranking across multiple directories.
- `crates/state`: Persistent store for manifests and mirrors.
- `crates/subagents`: Shared subagent runtime and backends for Codex/Claude delegation.

## Skill Discovery

Skills are discovered by searching through a prioritized sequence of directories:

1. `~/.codex/skills`
2. `~/.codex/skills-mirror` (a mirror of Claude skills)
3. `~/.claude/skills`
4. `~/.agent/skills`

You can customize discovery priority using a `~/.codex/skills-manifest.json` file:

```json
{ "priority": ["codex","mirror","claude","agent"], "cache_ttl_ms": 60000 }
```

## Typical Workflows

### Validate Skills for Codex Compatibility

```bash
skrills validate --target codex
skrills validate --target codex --autofix  # Auto-add missing frontmatter
```

### Analyze Skill Token Usage

```bash
skrills analyze --min-tokens 1000 --suggestions
```

### Sync All Configurations from Claude to Codex

```bash
skrills sync-all --from claude --skip-existing-commands
```

### Start MCP Server

```bash
skrills serve
```

### Launch a Mirrored Agent

```bash
skrills agent codex-dev
```

## Installation

- **macOS / Linux**: `curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh`
- **Windows PowerShell**: See installation guide for PowerShell command.
- **crates.io**: `cargo install skrills`
- **From source**: `cargo install --path crates/cli --force`
