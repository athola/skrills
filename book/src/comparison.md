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
Skrills validates skills against two targets with distinct requirements. Claude Code is permissive, accepting any markdown with optional frontmatter. Codex CLI is strict, requiring YAML frontmatter with `name` and `description` fields. To bridge this gap, the `--autofix` flag automatically derives and adds missing frontmatter from the file path and content.

### Token Analysis
Beyond simple counting, Skrills analyzes skills to estimate token usage and track dependencies. It provides optimization suggestions for large skills, helping you keep your context window efficient.

### Bidirectional Sync
Unlike tools that only copy files one way, Skrills supports full bidirectional sync for skills, commands, MCP servers, and preferences. It uses byte-for-byte command sync to preserve non-UTF-8 content and includes a `--skip-existing-commands` flag to protect your local customizations from being overwritten.

## Planned Improvements
We plan to add signed artifacts and version pinning for synced skills to improve security and reproducibility. We also aim to improve Windows path detection and default configurations, as well as implement skill dependency resolution to better manage complex skill sets.

## Summary

Skrills is a skills support engine focused on quality and portability. It validates skills for cross-CLI compatibility, analyzes token usage, and synchronizes configurations bidirectionally. The MCP server enables integration with both Claude Code and Codex CLI environments.
