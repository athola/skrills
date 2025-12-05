# Project Overview

`skrills` is an MCP server that gives AI agents access to local `SKILL.md` definitions. Its capabilities include mirroring external skill repositories, dynamically filtering skills based on prompt relevance, and integrating these skills into Claude Code through custom hooks. Furthermore, `skrills` maintains an updated `AGENTS.md` file, which lists all available skills.

## Capabilities

- **MCP Server**: Operates over standard I/O (stdio), providing endpoints for managing skills and tools.
- **Skill Discovery**: Locates skills across multiple predefined directories (including Codex, Claude mirror, Claude, and Agent skill locations). It resolves potential conflicts by de-duplicating entries based on a clearly defined priority system.
- **Autoloading**: Dynamically filters skills based on their relevance to the current prompt, supports manual pinning, and automatically prioritizes frequently used skills. This feature includes detailed diagnostics and content truncation to ensure that skills fit within predefined byte budgets.
- **Synchronization Utilities**: Offers tools to mirror Claude skills to Codex, export skill listings to [`AGENTS.md`](AGENTS.md) (in XML format), and provides a Terminal User Interface (TUI) for interactive skill pinning.
- **Installation**: Has automated installers compatible with `curl` (for macOS/Linux) and PowerShell (for Windows). These installers configure Claude Code with hooks for automatic skill injection. Alternatively, `skrills` can be built directly from source using `cargo`. The [`Makefile`](Makefile) includes targets for various demonstration purposes.