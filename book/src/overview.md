# Overview

`codex-mcp-skills` serves as an MCP server, integrating local `SKILL.md` files for use by AI agents. Its core functionality includes mirroring external skill repositories, dynamically filtering and pinning skills based on each prompt, and providing autoload context for platforms like Codex. The project also ensures `AGENTS.md` remains synchronized with a machine-readable list of all accessible skills.
...
## Capabilities

`codex-mcp-skills` provides:

-   **MCP Server**: Operates over standard I/O (stdio) with dedicated endpoints for managing resources and tools.
-   **Skill Discovery**: Discovers skills across various roots (Codex, Claude mirror, Claude, and Agent skill directories) using a priority-aware mechanism. This process removes duplicate entries to maintain a focused and relevant skill set.
-   **Autoloading**: Filters skills based on prompt content, supports manual pinning, and automatically pins frequently used skills. It provides detailed diagnostics and manages skill inclusion through byte budget truncation.
-   **Synchronization Utilities**: Includes tools for mirroring Claude skills to Codex, exporting skill lists to `AGENTS.md` in XML format, and a Terminal User Interface (TUI) for interactive skill pinning.
-   **Flexible Installation**: Offers standalone installers via `curl` (for macOS/Linux) and `PowerShell` (for Windows), in addition to standard `cargo` builds and a `Makefile` for demonstrations.
