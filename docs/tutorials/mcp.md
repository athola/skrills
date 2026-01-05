# MCP Integration: Testing with Claude Code

This tutorial shows how to use Skrills as an MCP server and **test MCP tool functionality inside Claude Code CLI**.

![MCP Demo](../../assets/gifs/mcp.gif)

## Overview

Skrills can run as an MCP (Model Context Protocol) server, providing:
- **Skills as Resources**: Each discovered skill is exposed as an MCP resource
- **search-skills-fuzzy**: Fuzzy search skills by name or content
- **resolve-dependencies**: Find skill dependencies and dependents
- **recommend-skills**: Get related skill recommendations
- **skill-metrics**: View aggregate skill statistics

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

### Step 2: Test MCP Resources in Claude Code CLI

Use `claude -p` (print mode) to test MCP features non-interactively:

```bash
# List available skill resources
claude -p "List all MCP resources from the skrills server" --model haiku

# Search for skills
claude -p "Use mcp__skrills__search-skills-fuzzy with query 'coding'" --model haiku
```

This command:
- Starts Claude Code in non-interactive mode (`-p`)
- Sends a prompt that invokes MCP resources/tools
- Uses the haiku model for faster/cheaper testing
- Returns discovered skills and their content

### Step 3: Interactive Testing

Start an interactive Claude Code session and ask it to use MCP tools:

```bash
claude
```

Then try prompts like:
- "Search for skills related to testing"
- "Load the debug-helper skill using MCP"
- "Show me skill recommendations for API work"

## Available MCP Features

### Resources

Skills are exposed as MCP resources with URIs like `skill://skrills/{source}/{name}`:
- Use `ListMcpResourcesTool` to list available skills
- Use `ReadMcpResourceTool` to read skill content

### Tools

When running as an MCP server, skrills provides these tools:

| Tool | Description |
|------|-------------|
| `search-skills-fuzzy` | Fuzzy search skills by name or content |
| `recommend-skills` | Get related skill recommendations based on dependencies |
| `recommend-skills-smart` | AI-powered skill recommendations |
| `resolve-dependencies` | Find dependencies or dependents for a skill |
| `skill-metrics` | View aggregate statistics about discovered skills |
| `validate-skills` | Validate skills for compatibility |
| `analyze-skills` | Analyze skills for token usage and optimization |

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
