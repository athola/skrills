# Comparison to Similar Projects

This table compares `skrills` with other approaches for managing and deploying skills across agents.

| Project type | What it ships | Transport/runtime | Automation surface | Strengths | Gaps vs skrills |
| --- | --- | --- | --- | --- | --- |
| **skrills** | MCP server + CLI, hooks for Codex/Claude, skill sync utilities | MCP over stdio; one binary | Hooks, CLI, release artifacts per target | Unified MCP layer, cross-agent sync, autoload hook, TUI/CLI parity | — |
| Static skill bundles | Ready-to-use skill files for manual placement | None (static skill files) | Manual copy into skill dir | Simple drop-in content | No MCP server; no Codex bridging; no installer |
| CI doc/render pipelines | Build-time converters of SKILL-like docs to prompt text | Build-time only | CI (Actions/pipelines) | Automates documentation rendering | No runtime server, discovery, or sync; prompt-only |
| Shared rules repositories | Curated rules/skills stored in a repo | N/A (static) | Manual consumption | Common baseline ruleset | No installer, no automation, no MCP bridge |
| Local skill sync CLIs | CLI/TUI to sync/rank local skills and mirror directories | Local sync; no MCP | CLI/TUI | Good local curation & mirroring | No MCP server; no Codex hook; limited to file sync |
| Tutorials/how-to guides | Guidance on authoring skills | N/A | Article/docs | Educational | No tooling; manual steps only |

Where we can still improve:
- Ship richer health diagnostics in the MCP server (latency, discovery stats).
- Offer optional signed artifacts and stronger version pinning for synced skills.
- Make Windows defaults and path detection even smoother.

## Summary

The main difference between `skrills` and other similar projects is its focus on providing a runtime MCP server that can be used by both Codex and Claude. Other projects in this space tend to be either static collections of skills, CI tools for rendering documentation, or local synchronization tools that don't have an MCP layer.

`skrills` also provides installers for pre-built binaries, which means you don't have to build it from source or manually copy files. The autoload hook and terminal user interface are designed to make it easy to integrate `skrills` into your workflow.
