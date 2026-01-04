# MCP Integration: Claude Code & Codex

This tutorial shows how to use Skrills as an MCP server to provide dynamic skill loading and management for both Claude Code and Codex.

![MCP Demo](../../assets/gifs/mcp.gif)

## Overview

Skrills can run as an MCP (Model Context Protocol) server, providing tools for:
- **autoload-snippet**: Load skill content dynamically
- **search-skills**: Search skills by name or content
- **resolve-dependencies**: Find skill dependencies
- **recommend-skills**: Get skill recommendations

The same MCP server works with both Claude Code and Codex.

## Setup

### For Claude Code

Add to your `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "skrills": {
      "command": "skrills",
      "args": ["serve"]
    }
  }
}
```

### For Codex

Add to your `~/.codex/mcp.json`:

```json
{
  "mcpServers": {
    "skrills": {
      "command": "skrills",
      "args": ["serve"]
    }
  }
}
```

## Using the MCP Tools

### Check MCP Configuration

Verify the MCP server is configured:

```bash
cat ~/.claude/settings.json | grep -A 5 'mcpServers'
```

### Diagnose MCP Setup

Run the doctor command to check configuration:

```bash
skrills doctor
```

This shows:
- Whether skrills is configured as an MCP server
- Platform-specific configuration status
- Any issues that need fixing

### Sync MCP Configurations

Copy MCP server settings between platforms:

```bash
skrills sync-mcp-servers --dry-run
```

Use without `--dry-run` to actually sync:

```bash
skrills sync-mcp-servers
```

## Available MCP Tools

When running as an MCP server, skrills provides these tools:

| Tool | Description |
|------|-------------|
| `autoload-snippet` | Load skill content by URI |
| `search-skills` | Search skills by name/content |
| `resolve-dependencies` | Find dependencies for a skill |
| `recommend-skills` | Get related skill recommendations |

## Tips

- Use `skrills serve --help` to see server options
- The server runs over stdio by default (MCP standard)
- Use `--dry-run` to preview MCP sync changes
- Check `skrills doctor` if MCP tools aren't available

## Requirements

- Skrills installed and in PATH
- Claude Code or Codex configured for MCP
- MCP client support in your Claude application
