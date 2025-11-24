# Comparison to Nearby Projects

The table below expands on how codex-mcp-skills differs from other public efforts to share or operate “skills” across agents—without naming projects directly.

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

## Key takeaways
- Our differentiator is the MCP server that exposes skills uniformly to Codex and Claude while keeping stdio transport. Competing projects are either static skill packs, CI renderers, or local sync tools without an MCP layer.
- Release artifacts and installers are turnkey (`curl | sh` / `irm | iex`) with per-target binaries; others rely on manual copies or Cargo installs.
- Autoload hook plus TUI provides end-user ergonomics; others focus on content, not runtime integration.
