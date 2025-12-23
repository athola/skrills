# What is Skrills?

Skrills helps you manage AI assistant skills across different tools. If you use Claude Code, Codex CLI, or both, skrills keeps your skills organized, validated, and synchronized.

## The Problem Skrills Solves

AI coding assistants like Claude Code and Codex CLI use "skills" — instructions that teach them specific behaviors. But these tools store skills differently and have different requirements. Skrills bridges this gap.

**Without skrills:**
- Skills written for Claude Code may not work in Codex CLI
- You manually copy skills between tools
- Large skills slow down your assistant without warning
- You discover compatibility problems only when skills fail

**With skrills:**
- Validate skills work across both tools before problems occur
- Sync skills automatically with one command
- Identify which skills consume the most tokens
- Get suggestions to optimize slow or bloated skills

## Core Features

### Validate Skills

Check that your skills meet requirements for Claude Code and Codex CLI:

```bash
skrills validate --target codex              # Check Codex compatibility
skrills validate --target codex --autofix    # Fix issues automatically
```

### Analyze Token Usage

Find skills that consume excessive context:

```bash
skrills analyze --min-tokens 1000 --suggestions
```

### Sync Between Tools

Copy all your settings from one tool to another:

```bash
skrills sync-all --from claude
```

### Run as MCP Server

Expose skrills capabilities to your AI assistant:

```bash
skrills serve
```

## Quick Start

1. **Install skrills:**
   ```bash
   curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
   ```

2. **Check your skills work:**
   ```bash
   skrills validate
   ```

3. **Sync from Claude to Codex (if you use both):**
   ```bash
   skrills sync-all --from claude
   ```

## Key Concepts

### Skills

Skills are markdown files (named `SKILL.md`) that teach your AI assistant specific behaviors. They contain instructions, examples, and metadata. Skrills discovers skills from these directories:

1. `~/.codex/skills` — Codex CLI skills
2. `~/.claude/skills` — Claude Code skills
3. `~/.agent/skills` — Universal agent skills

### MCP (Model Context Protocol)

MCP lets AI assistants communicate with external tools. Skrills runs as an MCP server, exposing its capabilities directly to Claude Code or Codex CLI. This means your assistant can validate skills, analyze token usage, or sync configurations without you running commands manually.

### Validation Targets

Claude Code and Codex CLI have different requirements:
- **Claude Code** — Permissive. Most valid markdown works.
- **Codex CLI** — Strict. Requires specific frontmatter fields.

Use `--target both` to ensure skills work everywhere.

## Architecture (For the Curious)

Skrills is built as a Rust workspace with focused crates:

| Crate | Purpose |
|-------|---------|
| `server` | MCP server and CLI interface |
| `validate` | Skill validation for both platforms |
| `analyze` | Token counting and optimization hints |
| `intelligence` | Smart recommendations and skill creation |
| `sync` | Bidirectional sync between Claude and Codex |
| `discovery` | Find and rank skills across directories |
| `subagents` | Delegate tasks to Claude or Codex |

## Next Steps

- **New to skrills?** Start with the [Installation Guide](installation.md)
- **Using both CLIs?** See the [Sync Guide](sync-guide.md)
- **Optimizing skills?** Check [MCP Token Optimization](mcp-token-optimization.md)
- **Have questions?** Browse the [FAQ](faq.md)
