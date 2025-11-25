# Installation

This guide provides instructions on how to install `codex-mcp-skills`.

## One-Liners (Recommended)

These commands provide a quick way to install `codex-mcp-skills` on your system.

```bash
# macOS / Linux
curl -LsSf https://raw.githubusercontent.com/athola/codex-mcp-skills/HEAD/scripts/install.sh | sh

# Windows
powershell -ExecutionPolicy Bypass -NoLogo -NoProfile -Command "Remove-Item alias:curl -ErrorAction SilentlyContinue; iwr https://raw.githubusercontent.com/athola/codex-mcp-skills/HEAD/scripts/install.ps1 -UseBasicParsing | iex"
```

You can customize the installation using these environment variables:
- `CODEX_SKILLS_GH_REPO`: Overrides the default GitHub repository (`athola/codex-mcp-skills`). Useful if you are using a fork.
- `CODEX_SKILLS_VERSION`: Specifies a particular version to install (e.g., `1.0.0`). Uses the latest stable version by default.
- `CODEX_SKILLS_BIN_DIR`: Sets the installation directory for the binary (defaults to `~/.codex/bin`).
- `CODEX_SKILLS_TARGET`: Forces installation for a specific target triple (e.g., `x86_64-unknown-linux-gnu`).

Additionally, the `--local` flag can be used to build and install from your current local checkout using `cargo`, instead of downloading a pre-built release.

## From Source

To install directly from the source code, use `cargo`:

```bash
cargo install --path crates/cli --force
```

## Hook & MCP Registration

The installer automatically sets up the necessary hooks and registers the MCP server:

```bash
./scripts/install-codex-skills.sh [--universal] [--universal-only]
```

-   **Hook**: The `prompt.on_user_prompt_submit` hook is written to `~/.codex/hooks/codex/`. This hook allows `codex-mcp-skills` to process prompts.
-   **MCP Server Registration**: The MCP server is registered in `~/.codex/mcp_servers.json`. The installer ensures `type = "stdio"` is correctly configured, as required by newer Codex MCP clients.
-   `--universal`: This flag also mirrors skills into `~/.agent/skills`, making them available for other agents.
-   `--universal-only`: Performs only the mirroring step without installing the main binary or hooks.

## Make Targets

The `Makefile` provides additional targets for installation and development:

```bash
make build         # Perform a release build of the project.
make serve-help    # Display help for the 'serve' command.
make emit-autoload # Emit an autoload snippet.
make demo-all      # Run a full CLI demonstration in a sandboxed environment.
make book          # Build the mdBook documentation and open it in your default browser.
make book-serve    # Start a live-reloading server for the mdBook documentation on localhost:3000.
```
