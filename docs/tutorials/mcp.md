# MCP Integration: Testing with Claude Code

This tutorial shows how to use Skrills as an MCP server and **test MCP tool functionality inside Claude Code CLI**.

![MCP Demo](../../assets/gifs/mcp.gif)

## Overview

Skrills can run as an MCP (Model Context Protocol) server, providing tools for:
- **autoload-snippet**: Load skill content dynamically
- **search-skills**: Search skills by name or content
- **resolve-dependencies**: Find skill dependencies
- **recommend-skills**: Get skill recommendations

## Setup

### For Claude Code

Add to your `~/.claude/settings.local.json` (or `~/.claude/settings.json`):

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

## Testing MCP Inside Claude Code

### Step 1: Verify MCP Connection

Check that the skrills MCP server is connected:

```bash
claude mcp list
```

You should see output like:

```
skrills: skrills serve - ✓ Connected
```

### Step 2: Test MCP Tools in Claude Code CLI

Use `claude -p` (print mode) to test MCP tools non-interactively:

```bash
claude -p "Use the mcp__skrills__autoload-snippet tool to load the skill from skrills://skill/coding-assistant and show its content" --model haiku
```

This command:
- Starts Claude Code in non-interactive mode (`-p`)
- Sends a prompt that invokes the MCP tool
- Uses the haiku model for faster/cheaper testing
- Displays the skill content loaded via MCP

### Step 3: Interactive Testing

Start an interactive Claude Code session and ask it to use MCP tools:

```bash
claude
```

Then try prompts like:
- "Search for skills related to testing"
- "Load the debug-helper skill using MCP"
- "Show me skill recommendations for API work"

## Available MCP Tools

When running as an MCP server, skrills provides these tools to Claude Code:

| Tool | Description | Example Usage |
|------|-------------|---------------|
| `mcp__skrills__autoload-snippet` | Load skill content by URI | `skrills://skill/coding-assistant` |
| `mcp__skrills__search-skills` | Search skills by name/content | "search for testing skills" |
| `mcp__skrills__resolve-dependencies` | Find dependencies for a skill | "what does debug-helper depend on" |
| `mcp__skrills__recommend-skills` | Get related skill recommendations | "recommend skills for API work" |

## Troubleshooting

### MCP Server Not Connecting

1. Check if skrills is in PATH:
   ```bash
   which skrills
   ```

2. Verify the configuration:
   ```bash
   cat ~/.claude/settings.local.json | grep -A 5 'mcpServers'
   ```

3. Run the doctor command:
   ```bash
   skrills doctor
   ```

### Tools Not Available

If Claude Code can't find the MCP tools:

1. Restart Claude Code to reload MCP configuration
2. Check server logs with `--mcp-debug`:
   ```bash
   claude --debug
   ```

## Tips

- Use `claude -p` for scripted testing of MCP tools
- Add `--model haiku` for faster/cheaper test runs
- Use `--max-budget-usd 0.10` to limit API costs during testing
- Check `claude mcp list` to verify server connection status
- Use `skrills serve --help` to see MCP server options

## Requirements

- Skrills installed and in PATH
- Claude Code CLI installed
- Valid Anthropic API key (for `claude -p` testing)
