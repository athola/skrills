# Overview

`codex-mcp-skills` acts as an MCP server, transforming local `SKILL.md` files into readily available resources and tools for AI agents. Its core functionality includes mirroring external skill repositories, dynamically filtering and pinning relevant skills based on each prompt, and providing comprehensive autoload context for platforms like Codex. The project also ensures that `AGENTS.md` remains synchronized with a machine-readable list of all accessible skills.

## Key Capabilities

`codex-mcp-skills` offers the following key capabilities:

-   **MCP Server**: Operates over standard I/O (stdio) and provides dedicated endpoints for managing resources and tools.
-   **Intelligent Skill Discovery**: Employs a priority-aware discovery mechanism across various skill roots (Codex, Claude mirror, Claude, and Agent skill directories), effectively suppressing duplicates to ensure a clean and relevant skill set.
-   **Advanced Autoloading**: Features a robust autoload tool that filters skills based on prompt content, supports manual pinning, automatically pins skills from historical usage, provides detailed diagnostics, and manages skill inclusion through byte budget truncation.
-   **Synchronization Utilities**: Includes helpers for mirroring Claude skills to Codex, exporting skill lists to `AGENTS.md` in XML format, and a Terminal User Interface (TUI) for interactive skill pinning.
-   **Flexible Installation**: Offers standalone installers via `curl` (for macOS/Linux) and `PowerShell` (for Windows), alongside standard `cargo` builds and a `Makefile` configured for demonstrations.
