# Installation Guide

## crates.io (Recommended)

To install the binary from `crates.io`, run the following command:

```bash
cargo install skrills
```

## One-Liners (Release Artifacts)

```bash
# macOS / Linux
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh

# Windows
powershell -ExecutionPolicy Bypass -NoLogo -NoProfile -Command "Remove-Item alias:curl -ErrorAction SilentlyContinue; iwr https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.ps1 -UseBasicParsing | iex"
```

You can customize the installation using environment variables or flags:
- `SKRILLS_GH_REPO`: Overrides the default GitHub repository (`athola/skrills`).
- `SKRILLS_VERSION`: Installs a specific version (use the tag without the leading `v` prefix).
- `SKRILLS_BIN_DIR`: Sets the installation directory (defaulting to `~/.codex/bin`).
- `SKRILLS_TARGET`: Sets the Rust target triple (e.g., `x86_64-unknown-linux-gnu`) for platform-specific builds.
- `SKRILLS_CLIENT`: Forces the installer to configure for a client: `codex` or `claude`. If not set, the installer attempts auto-detection based on the presence of `~/.claude` or `~/.codex` directories.
- `SKRILLS_BASE_DIR`: Overrides the client configuration root directory.
- `SKRILLS_NO_MIRROR`: When set to `1`, skips the post-install mirror step that copies Claude assets into Codex (Codex installs only).
- `--install-path <PATH>`: Sets the installation directory for the binaries.
- `--client <codex|claude>`: Forces the installer to target a client type: `codex` or `claude`.
- `--base-dir <PATH>`: Overrides the root directory for client configuration files.
- `--local`: Builds `skrills` from a local source checkout instead of downloading release artifacts.

## Common Scenarios

```bash
# Codex Default (auto-detected from ~/.codex):
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh

# Claude (auto-detected from ~/.claude):
SKRILLS_BIN_DIR="$HOME/.claude/bin" \
  curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh

# Explicit Claude setup with custom base directory:
SKRILLS_CLIENT=claude SKRILLS_BASE_DIR=/tmp/claude-demo \
  curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
```

## From Source

To install from source (requires Rust toolchain), run:

```bash
cargo install --path crates/cli --force
```

## Hook & MCP Registration

The installer configures client-specific hooks (where applicable) and registers the MCP server. On Codex installs it also mirrors Claude assets into Codex unless `SKRILLS_NO_MIRROR=1`. If `~/.claude` is missing, the installer skips mirroring and prints a reminder to run `skrills mirror` later once Claude assets exist. This process starts with:

```bash
./scripts/install-skrills.sh [--universal] [--universal-only]
```

- **Hooks**: For Claude, the installer creates the [`~/.claude/hooks/prompt.on_user_prompt_submit`](~/.claude/hooks/prompt.on_user_prompt_submit) file. For Codex, hooks are not currently implemented.
- **MCP Registration**: During the registration process, the installer updates both [`~/.codex/mcp_servers.json`](~/.codex/mcp_servers.json) and [`~/.codex/config.toml`](~/.codex/config.toml).
- **Legacy Cleanup**: Removes obsolete `codex-mcp-skills` binaries and their associated configuration entries.
- **Universal Skill Mirroring**: If enabled, this mirrors skills to the `~/.agent/skills` directory, making them accessible across various agent environments.

## Make Targets

For convenience during development, the `Makefile` provides several common targets:

```bash
make build         # Release build
make serve-help    # Help for 'serve'
make emit-autoload # Emit autoload snippet
make demo-all      # Full CLI demo
make book          # Build mdBook
make book-serve    # Live mdBook on localhost:3000
```
