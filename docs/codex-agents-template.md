# Skrills Integration Guide for Codex Agents

## Overview

Skrills is a skills support engine that validates, analyzes, and syncs skills between Claude Code and Codex CLI. This template describes how to integrate skrills into Codex agent workflows.

## Available MCP Tools

When skrills is registered as an MCP server, the following tools are available:

### Validation Tools

- **`validate-skills`**: Validate skills for Claude Code and/or Codex CLI compatibility
  ```json
  {
    "name": "validate-skills",
    "arguments": {
      "target": "codex",
      "autofix": true
    }
  }
  ```

- **`analyze-skills`**: Analyze skills for token usage and optimization opportunities
  ```json
  {
    "name": "analyze-skills",
    "arguments": {
      "min_tokens": 1000,
      "suggestions": true
    }
  }
  ```

### Sync Tools

- **`sync-from-claude`**: Copy Claude skills into Codex discovery root (`~/.codex/skills/`)
- **`sync-skills`**: Sync skills between agents
- **`sync-commands`**: Sync slash commands
- **`sync-mcp-servers`**: Sync MCP server configurations
- **`sync-preferences`**: Sync preferences
- **`sync-all`**: Sync all configurations
- **`sync-status`**: Preview sync changes (dry run)

## Recommended Workflows

### Skill Quality Assurance

When the user asks about skill compatibility or quality:

1. Run `validate-skills` with `target: "both"` to check cross-CLI compatibility
2. If issues found, suggest using `autofix: true`
3. Run `analyze-skills` to identify optimization opportunities

### Configuration Sync

When the user wants to sync configurations:

1. Run `sync-status` to preview changes
2. Explain what will be synced
3. Run `sync-all` with appropriate options

### Best Practices

- Always preview with `sync-status` before running `sync-all`
- Use `skip_existing_commands: true` to preserve local customizations
- Run `validate-skills` after syncing to verify compatibility
