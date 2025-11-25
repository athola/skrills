# Comparison to Similar Projects

The table below expands on how codex-mcp-skills differs from other public efforts to share or operate “skills” across agents.

| Project type | What it ships | Transport/runtime | Automation surface | Strengths | Gaps vs codex-mcp-skills |
| --- | --- | --- | --- | --- | --- |
| **codex-mcp-skills** | MCP server + CLI, hooks for Codex/Claude, skill sync utilities | MCP over stdio; one binary | Hooks, CLI, release artifacts per target | Unified MCP layer, cross-agent sync, autoload hook, TUI/CLI parity | — |
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

`codex-mcp-skills` is primarily differentiated by its MCP server, which exposes skills to both Codex and Claude over a standard stdio interface. Other projects in this space tend to be static collections of skills, CI tools for rendering documentation, or local synchronization tools without an MCP layer.

The project also provides convenient installers (`curl | sh` and `irm | iex`) for per-target binaries, in contrast to projects that require manual copying of files or installation via `cargo`.

Finally, the autoload hook and terminal user interface are focused on runtime integration and ease of use, whereas many other projects focus more on the content of the skills themselves.
