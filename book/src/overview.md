# Overview

`skrills` is an MCP server that makes local `SKILL.md` files available to AI agents. It can mirror skill repositories, filter skills based on the current prompt, and automatically provide context for platforms like Codex. It also keeps your `AGENTS.md` file up to date with a list of all available skills.
...
## Capabilities

`skrills` includes the following features:

-   **MCP Server**: A server that runs over standard I/O (stdio) and provides endpoints for managing skills and tools.
-   **Skill Discovery**: `skrills` finds skills in several default directories (Codex, Claude mirror, Claude, and Agent) and de-duplicates them based on a priority system.
-   **Autoloading**: It filters skills based on the content of your prompt, allows you to manually pin skills, and automatically pins skills that you use frequently. It also provides detailed diagnostics and truncates skills to fit within a byte budget.
-   **Synchronization Utilities**: It includes tools for mirroring Claude skills to Codex, exporting a list of skills to `AGENTS.md` in XML format, and a Terminal User Interface (TUI) for pinning skills.
-   **Installation**: You can install `skrills` using `curl` (for macOS/Linux), `PowerShell` (for Windows), or by building from source with `cargo`. A `Makefile` is also provided for running demonstrations.
