# Project Comparison

This table compares `skrills` against alternative approaches for managing and deploying skills across Claude Code and Codex CLI.

| Project Type | Key Components | Transport/Runtime | Automation Interface | Key Strengths | Distinguishing Gaps (vs. `skrills`) |
|---|---|---|---|---|---|
| **skrills** | MCP server, CLI, validation/analysis crates, skill synchronization utilities | MCP over stdio; single binary | CLI, MCP tools, release artifacts per target | Unified MCP layer, cross-agent synchronization, skill validation with autofix, token analysis, TUI/CLI feature parity | — |
| Static skill bundles | Ready-to-use skill files | None (static) | Manual copy | Straightforward, drop-in content deployment | Lacks validation, analysis, or synchronization. No MCP server or Codex bridging. |
| CI doc/render pipelines | Build-time converters | Build-time only | CI (GitHub Actions, custom pipelines) | Automates documentation rendering | No runtime server, skill discovery, or synchronization; limited to prompt-only operations. |
| Shared rules repositories | Curated collections of rules and skills | Not applicable (static) | Manual consumption | Provides common baseline ruleset | Lacks installer, automation, or MCP bridge. |
| Local skill sync CLIs | CLI or TUI for local skill synchronization | Local synchronization only; no MCP | CLI/TUI | Allows effective local curation and mirroring | No MCP server, no validation/analysis, limited to basic file sync. |
| Tutorials/how-to guides | Instructional content for authoring skills | Not applicable | Article/docs | Educational | Lacks integrated tooling; relies on manual steps. |

## Core Differentiators

### Validation Engine

Skrills validates skills against two targets:
- **Claude Code**: Permissive - accepts any markdown with optional frontmatter
- **Codex CLI**: Strict - requires YAML frontmatter with `name` and `description`

The `--autofix` flag automatically adds missing frontmatter.

### Token Analysis

Skrills analyzes skills for:
- Token count estimation
- Dependency tracking
- Optimization suggestions for large skills

### Bidirectional Sync

Unlike tools that only copy files one way, skrills supports full bidirectional sync:
- Skills, commands, MCP servers, and preferences
- Byte-for-byte command sync preserves non-UTF-8 content
- `--skip-existing-commands` protects local customizations

## Areas for Improvement

- Add signed artifacts and version pinning for synced skills
- Improve Windows path detection and default configurations
- Add skill dependency resolution

## Summary

Skrills is a skills support engine focused on quality and portability. It validates skills for cross-CLI compatibility, analyzes token usage, and synchronizes configurations bidirectionally. The MCP server enables integration with both Claude Code and Codex CLI environments.
