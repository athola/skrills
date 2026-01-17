# What is Skrills?

Skrills validates, analyzes, and synchronizes skills across Claude Code and Codex CLI. It resolves compatibility issues between the two environments and provides tools to manage context usage effectively.

## The Problem

Claude Code and Codex CLI both use markdown-based "skills," but they enforce different requirements. Claude Code has a permissive structure, while Codex CLI demands strict YAML frontmatter and imposes character limits. Skills written for one tool often fail in the other.

Skrills solves this friction by validating frontmatter and schema compliance, syncing configurations and skills bidirectionally, and analyzing token usage to prevent context overflow.

## Core Features

### Validate Skills
The validation engine checks compliance with Codex's strict requirements. You can automatically fix missing frontmatter by running:

```bash
skrills validate --target codex --autofix
```

### Analyze Token Usage
To prevent context window exhaustion, Skrills identifies heavy skills that might need optimization:

```bash
skrills analyze --min-tokens 1000 --suggestions
```

### Sync Between Tools
Skrills mirrors your configuration between environments, verifying your tools are available everywhere:

```bash
skrills sync-all --from claude
```

### Run as MCP Server
You can expose these capabilities directly to your AI assistant by running Skrills as an MCP server:

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
Skills are `SKILL.md` files containing instructions and metadata. Skrills scans for these files in `~/.codex/skills`, `~/.claude/skills`, and the universal `~/.agent/skills` directory.

### MCP (Model Context Protocol)
Skrills implements the Model Context Protocol, allowing assistants to invoke its tools (validation, analysis, sync) directly during a session.

### Validation Targets
Validation rules depend on the target. Claude Code is permissive and accepts any markdown with optional frontmatter. Codex CLI is strict, requiring `name` and `description` fields in the YAML frontmatter.

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
