# Project Overview

`skrills` is an MCP server that gives AI agents access to local `SKILL.md` definitions. Its capabilities include mirroring external skill repositories, dynamically filtering skills based on prompt relevance, and integrating these skills into Claude Code through custom hooks. It also keeps `AGENTS.md` current, launches mirrored agent specs via `skrills agent`, synchronizes commands/preferences between Codex and Claude, and provides a subagents runtime for multi-agent coordination.

## Capabilities

- **MCP Server**: Operates over standard I/O (stdio), providing endpoints for managing skills and tools.
- **Skill Discovery**: Locates skills across multiple predefined directories (including Codex, Claude mirror, Claude, and Agent skill locations). It resolves potential conflicts by de-duplicating entries based on a clearly defined priority system.
- **Autoloading**: Dynamically filters skills based on their relevance to the current prompt, supports manual pinning, and automatically prioritizes frequently used skills. This feature includes detailed diagnostics and content truncation to ensure that skills fit within predefined byte budgets.
- **Subagents Runtime**: Provides MCP tools (`list_subagents`, `run_subagent`, `get_run_status`) for executing subagents with configurable backends (Claude-style or Codex-style). Supports async execution and secure transcript handling.
- **Cross-Agent Sync**: Introduces sync orchestration with `SyncOrchestrator` and adapters for Claude/Codex, enabling cross-agent skill synchronization via `skrills sync import/export/report`.
- **Synchronization Utilities**: Mirrors Claude assets (skills, agents, commands, preferences) into Codex via `mirror`, `sync`, and `sync-all`; exports skill listings to [`AGENTS.md`](AGENTS.md); provides a TUI for interactive pinning and mirroring. Command sync is byte-for-byte and respects `--skip-existing-commands` to avoid overwriting local files.
- **Installation**: Has automated installers compatible with `curl` (for macOS/Linux) and PowerShell (for Windows). These installers configure Claude Code with hooks for automatic skill injection. Alternatively, `skrills` can be built directly from source using `cargo`. The [`Makefile`](Makefile) includes targets for various demonstration purposes.
