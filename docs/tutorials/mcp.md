# MCP Integration: Using Skrills with Claude Code

This tutorial shows how to use Skrills as an MCP server with Claude Code CLI and other AI coding assistants.

![MCP Demo](../../assets/gifs/mcp.gif)

## Overview

Skrills runs as an MCP (Model Context Protocol) server, providing 24 tools for skill management:

- **Sync Tools**: Bidirectional sync between Claude, Codex, and Copilot
- **Validation Tools**: Validate skills for CLI compatibility
- **Intelligence Tools**: Smart recommendations, project analysis, skill creation
- **Trace Tools**: Skill loading instrumentation and debugging

## Setup

### Option 1: Claude Code Plugin (Recommended)

Installing as a plugin gives you direct access to MCP tools as callable functions (e.g., `mcp__plugin_skrills_skrills__validate-skills`).

**Step 1: Add skrills as a marketplace**

```bash
# From GitHub
claude plugin marketplace add athola/skrills

# Or from local clone
claude plugin marketplace add /path/to/skrills
```

**Step 2: Install the plugin**

```bash
claude plugin install skrills@skrills-marketplace
```

**Step 3: Restart Claude Code** to load the new MCP server.

After installation, tools appear as `mcp__plugin_skrills_skrills__<tool-name>`.

### Option 2: MCP Server (User-Level)

Add to `~/.claude/settings.local.json`:

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

### Option 3: MCP Server (Project-Level)

Add to `.mcp.json` in your project root:

```json
{
  "mcpServers": {
    "skrills": {
      "type": "stdio",
      "command": "skrills",
      "args": ["serve"]
    }
  }
}
```

### For Codex

Add to `~/.codex/mcp.json`:

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

## Using Skrills Tools

### Method 1: Direct Tool Calls (Plugin Installation)

When installed as a plugin, tools are directly callable functions:

```
# Tools appear as mcp__plugin_skrills_skrills__<tool-name>
mcp__plugin_skrills_skrills__validate-skills
mcp__plugin_skrills_skrills__sync-all
mcp__plugin_skrills_skrills__skill-metrics
```

This provides the same experience as Notion, Sentry, and other plugin MCP servers.

### Method 2: CLI Commands

The most reliable way to use skrills functionality is via CLI commands. Each MCP tool has a corresponding CLI command:

| MCP Tool | CLI Command | Example |
|----------|-------------|---------|
| `search-skills-fuzzy` | N/A (MCP only) | Use Method 1 or 3 |
| `validate-skills` | `skrills validate` | `skrills validate --target all` |
| `analyze-skills` | `skrills analyze` | `skrills analyze --min-tokens 500` |
| `skill-metrics` | `skrills metrics` | `skrills metrics` |
| `recommend-skills-smart` | `skrills recommend-skills-smart` | `skrills recommend-skills-smart` |
| `suggest-new-skills` | `skrills suggest-new-skills` | `skrills suggest-new-skills` |
| `sync-all` | `skrills sync-all` | `skrills sync-all --to codex` |
| `create-skill` | `skrills create-skill` | `skrills create-skill --name test` |

**Example: Validate skills for all CLIs**
```bash
skrills validate --target all --autofix
```

**Example: Get skill metrics**
```bash
skrills metrics --format json
```

**Example: Sync to Copilot**
```bash
skrills sync-all --to copilot --dry-run
```

### Method 3: Natural Language Prompts

When working in Claude Code, ask the model to use skrills tools with natural language:

```
# In an interactive Claude Code session:
"Use the skrills MCP server to search for skills related to testing"
"Ask skrills to recommend skills for this project"
"Use skrills to validate my skills for Codex compatibility"
```

### Method 4: MCP Resources

Skills are exposed as MCP resources. Use the built-in tools to access them:

```
# List skrills resources
Use ListMcpResourcesTool with server="skrills"

# Read a specific skill
Use ReadMcpResourceTool with server="skrills" and uri="skill://skrills/claude/my-skill"
```

## Available MCP Tools (24 Total)

Run `skrills serve --list-tools` to see all available tools:

### Sync Tools (9)

| Tool | Description |
|------|-------------|
| `sync-from-claude` | Copy skills from ~/.claude to ~/.codex |
| `sync-from-copilot` | Sync from GitHub Copilot CLI |
| `sync-to-copilot` | Sync to GitHub Copilot CLI |
| `sync-skills` | Sync SKILL.md files between agents |
| `sync-commands` | Sync slash commands |
| `sync-mcp-servers` | Sync MCP server configurations |
| `sync-preferences` | Sync preferences |
| `sync-all` | Sync everything |
| `sync-status` | Preview sync changes (dry run) |

### Validation Tools (2)

| Tool | Description |
|------|-------------|
| `validate-skills` | Validate for Claude/Codex/Copilot compatibility |
| `analyze-skills` | Analyze token usage and optimization |

### Intelligence Tools (6)

| Tool | Description |
|------|-------------|
| `recommend-skills` | Dependency-based recommendations |
| `recommend-skills-smart` | AI-powered recommendations |
| `analyze-project-context` | Analyze project languages/frameworks |
| `suggest-new-skills` | Identify gaps in skill library |
| `create-skill` | Create skill via GitHub/LLM |
| `search-skills-fuzzy` | Typo-tolerant skill search |
| `search-skills-github` | Search GitHub for skills |

### Trace Tools (4)

| Tool | Description |
|------|-------------|
| `skill-loading-status` | Check skill roots and instrumentation |
| `enable-skill-trace` | Install trace/probe skills |
| `disable-skill-trace` | Remove trace skills |
| `skill-loading-selftest` | Confirm skills are loading |

### Other Tools (2)

| Tool | Description |
|------|-------------|
| `resolve-dependencies` | Get transitive dependencies/dependents |
| `skill-metrics` | Aggregate skill statistics |

## Verifying MCP Connection

### Check Server Status

```bash
# Verify skrills is in PATH
which skrills

# List available MCP tools
skrills serve --list-tools

# Run diagnostics
skrills doctor
```

### In Claude Code

Use `/mcp` to see connected servers. You should see:
```
Reconnected to skrills.
```

## Troubleshooting

### MCP Server Not Connecting

1. **Check PATH**: Ensure `skrills` is in your PATH
   ```bash
   which skrills
   ```

2. **Verify configuration**: Check your MCP settings
   ```bash
   cat ~/.claude/settings.local.json | jq '.mcpServers.skrills'
   # or for project-level:
   cat .mcp.json | jq '.mcpServers.skrills'
   ```

3. **Run diagnostics**:
   ```bash
   skrills doctor
   ```

4. **Restart Claude Code** to reload MCP configuration

### Tools Not Working

If MCP tools don't respond:

1. **Use CLI instead**: Most MCP tools have CLI equivalents
   ```bash
   skrills validate --target codex
   skrills metrics
   ```

2. **Check server logs**: Enable debug mode
   ```bash
   RUST_LOG=debug skrills serve
   ```

3. **Test server directly**: Verify tools are advertised
   ```bash
   skrills serve --list-tools
   ```

## Tips

- **CLI is most reliable**: Use `skrills <command>` for guaranteed functionality
- **Natural language works**: Ask Claude to "use skrills to..." for MCP access
- **Check connection**: Use `/mcp` in Claude Code to verify connection
- **JSON output**: Add `--format json` to CLI commands for machine parsing
- **Dry run first**: Use `--dry-run` before sync operations

## Requirements

- Skrills installed and in PATH
- Claude Code CLI (for Claude integration)
- Codex CLI (for Codex integration, optional)
- GitHub Copilot CLI (for Copilot integration, optional)
