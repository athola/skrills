# What is Skrills?

Skrills validates, analyzes, and synchronizes skills across Claude Code, Codex CLI, GitHub Copilot CLI, and Cursor. It resolves compatibility issues between the four environments and provides tools to manage context usage effectively.

## The Problem

Claude Code, Codex CLI, GitHub Copilot CLI, and Cursor all use markdown-based "skills," but they enforce different requirements. Claude Code has a permissive structure, Codex CLI and Copilot CLI demand strict YAML frontmatter, and Cursor uses `.mdc` rule files with mode-specific fields like `globs` and `alwaysApply`. Skills written for one tool often fail in the others.

Skrills solves this friction by validating frontmatter and schema compliance, syncing configurations and skills between all four CLIs, and analyzing token usage to prevent context overflow.

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
skrills sync-from-claude    # Claude as source of truth
skrills sync-from-cursor    # Cursor as source of truth
skrills sync                # Bidirectional sync
```

### Run as MCP Server
You can expose these capabilities directly to your AI assistant by running Skrills as an MCP server, or launch the browser dashboard:

```bash
skrills serve                    # MCP server over stdio
skrills serve --http --open      # Browser dashboard with REST API
```

### HTML Portal
Skrills ships with a self-contained HTML portal (`skrills-portal.html`) that provides an interactive browser-based interface for exploring skills, validating content, analyzing tokens, converting between CLI formats, and browsing the full MCP tool and CLI command reference. The portal works offline on `file://` protocol and can be uploaded into AI application portals.

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
   skrills sync-from-claude
   ```

## Key Concepts

### Skills
Skills are `SKILL.md` files containing instructions and metadata. Skrills scans for these files in `~/.claude/skills`, `~/.codex/skills`, `~/.copilot/skills`, `~/.cursor/rules/`, and the universal `~/.agent/skills` directory.

### MCP (Model Context Protocol)
Skrills implements the Model Context Protocol, allowing assistants to invoke its tools (validation, analysis, sync) directly during a session.

### Validation Targets
Validation rules depend on the target. Claude Code is permissive and accepts any markdown with optional frontmatter. Codex CLI and Copilot CLI are strict, requiring `name` and `description` fields in the YAML frontmatter. Cursor uses `.mdc` rule files where the `description` field is required and mode is derived from `globs` or `alwaysApply` fields.

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
