# Project Comparison

This table compares `skrills` against alternative approaches for managing and deploying skills across various agents.

| Project Type | Key Components | Transport/Runtime | Automation Interface | Key Strengths | Distinguishing Gaps (vs. `skrills`) |
|---|---|---|---|---|---|
| **skrills** | MCP server, CLI, hooks for Codex/Claude, skill synchronization utilities | MCP over stdio; single binary | Hooks, CLI, release artifacts per target | Unified MCP layer, cross-agent synchronization, autoload hook integration, TUI/CLI feature parity | — |
| Static skill bundles | Ready-to-use skill files | None (static) | Manual copy | Straightforward, drop-in content deployment. | Lacks an MCP server, Codex bridging capabilities, and an integrated installer. |
| CI doc/render pipelines | Build-time converters. | Build-time only | Continuous Integration (e.g., GitHub Actions, custom pipelines). | Automates documentation rendering. | No runtime server, skill discovery, or synchronization features; limited to prompt-only operations. |
| Shared rules repositories | Curated collections of rules and skills in a central repository. | Not applicable (static) | Manual consumption | Provides a common, baseline ruleset for broad application. | Lacks an installer, automation capabilities, or an MCP bridge. |
| Local skill sync CLIs | Command-line interface (CLI) or Terminal User Interface (TUI) for synchronizing and ranking local skills. | Local synchronization only; no MCP | CLI/TUI | Allows effective local curation and mirroring of skills. | No MCP server, Codex hook integration, and limited to basic file synchronization. |
| Tutorials/how-to guides | Instructional content for authoring skills. | Not applicable | Article/docs | Educational. | Lacks integrated tooling, relying solely on manual steps. |

## Areas for Improvement
- Provide richer health diagnostics from the MCP server (latency, discovery stats).
- Add optional signed artifacts and stronger version pinning for synced skills.
- Improve default configurations and path detection on Windows.

## Summary

`skrills`'s main advantage is its runtime MCP server, with its support for both Codex and Claude integrations. Unlike many alternative solutions that often provide static skill collections, CI-based documentation tools, or local synchronization utilities, `skrills` integrates a full MCP layer. Also, pre-built binaries and installers make setup easier.