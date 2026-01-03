# What is Skrills?

Skrills validates, analyzes, and synchronizes skills across Claude Code and Codex CLI. It ensures compatibility between the two environments and helps manage context usage.

## The Problem

Claude Code and Codex CLI both use markdown-based "skills," but they have different requirements:
- **Claude Code**: Permissive structure.
- **Codex CLI**: Strict YAML frontmatter and character limits.

Skills written for one tool often fail in the other. Skrills solves this by:
- **Validating** frontmatter and schema compliance.
- **Syncing** configurations and skills bidirectionally.
- **Analyzing** token usage to prevent context overflow.

## Core Features

### Validate Skills

Check compliance and auto-fix frontmatter:

```bash
skrills validate --target codex --autofix
```

### Analyze Token Usage

Identify heavy skills:

```bash
skrills analyze --min-tokens 1000 --suggestions
```

### Sync Between Tools

Mirror configuration:

```bash
skrills sync-all --from claude
```

### Run as MCP Server

Expose these capabilities directly to the assistant:

```bash
skrills serve
```

## Quick Start

1. **Install:**
   ```bash
   curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
   ```

2. **Validate:**
   ```bash
   skrills validate
   ```

3. **Sync:**
   ```bash
   skrills sync-all --from claude
   ```

## Key Concepts

### Skills

Skills are `SKILL.md` files containing instructions and metadata. Skrills scans:
1. `~/.codex/skills`
2. `~/.claude/skills`
3. `~/.agent/skills` (Universal)

### MCP (Model Context Protocol)

Skrills implements the Model Context Protocol, allowing assistants to invoke its tools (validation, analysis, sync) directly during a session.

### Validation Targets

- **Claude Code**: Permissive.
- **Codex CLI**: Strict (requires `name`, `description` in frontmatter).

## Architecture

Skrills is a Rust workspace:

| Crate | Purpose |
|-------|---------|
| `server` | MCP server and CLI interface |
| `validate` | Skill validation |
| `analyze` | Token counting |
| `intelligence` | Recommendations and skill creation |
| `sync` | Bidirectional sync |
| `discovery` | Skill discovery |
| `subagents` | Task delegation |

## Next Steps

- [Installation Guide](installation.md)
- [Sync Guide](sync-guide.md)
- [MCP Token Optimization](mcp-token-optimization.md)
- [FAQ](faq.md)
