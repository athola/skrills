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

`skrills` stands out due to its MCP server, which exposes skills to both Codex and Claude over a standard stdio interface. Other projects in this area are typically static skill collections, CI tools for documentation rendering, or local synchronization tools lacking an MCP layer.

The project also provides direct installers (`curl | sh` and `irm | iex`) for per-target binaries, contrasting with projects that necessitate manual file copying or `cargo` installation.

The autoload hook and terminal user interface prioritize runtime integration and operational efficiency, whereas many other projects emphasize skill content creation.
