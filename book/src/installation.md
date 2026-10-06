# Installation Guide

## Quick Install (Recommended)

Most users should run this one-liner:

**macOS / Linux:**
```bash
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
```

**Windows PowerShell:**
```powershell
powershell -ExecutionPolicy Bypass -NoLogo -NoProfile -Command "iwr https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.ps1 -UseBasicParsing | iex"
```

The installer:
1. Downloads the correct binary for your system
2. Installs it to `~/.skrills/bin` (override with `SKRILLS_BIN_DIR`)
3. Runs `skrills setup --yes --client <client>` to register skrills as an MCP server, once for each client whose config directory (`~/.claude`, `~/.codex`, `~/.copilot`, `~/.cursor`) exists, or for Claude Code when none does
4. With `SKRILLS_UNIVERSAL=1`, also syncs your Claude skills into `~/.agent/skills`

## Verify Installation

```bash
skrills --version
skrills doctor        # Check configuration
```

## Alternative: Install from crates.io

If you have Rust installed:

```bash
cargo install skrills
```

## Alternative: Build from Source

Clone and build locally:

```bash
git clone https://github.com/athola/skrills.git
cd skrills
cargo install --path crates/cli --force
```

## Customizing Installation

The installer accepts environment variables to customize behavior:

| Variable | Purpose | Default |
|----------|---------|---------|
| `SKRILLS_CLIENT` | Client for `skrills setup`: `claude`, `codex`, `copilot`, `cursor`, `both` or `all` | Each client whose `~/.<client>` directory exists; `claude` if none does |
| `SKRILLS_BIN_DIR` | Where to install the binary | `~/.skrills/bin` |
| `SKRILLS_VERSION` | Install a specific version (tag without the leading `v`) | Latest |
| `SKRILLS_NO_HOOK` | Set to `1` to skip `skrills setup` | Unset |
| `SKRILLS_UNIVERSAL` | Set to `1` to also sync Claude skills into `~/.agent/skills` | Unset |
| `SKRILLS_TARGET` | Target triple of the release asset | Detected from `uname` |
| `SKRILLS_SKIP_CHECKSUM` | Set to `1` to install without verifying the release checksum | Unset |

Set the variables on the `sh` side of the pipe; a variable placed before `curl` reaches only `curl`.

### Examples

**Install for Claude Code only:**
```bash
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh \
  | SKRILLS_CLIENT=claude sh
```

**Install a specific version:**
```bash
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh \
  | SKRILLS_VERSION=0.4.0 sh
```

**Install the binary only, without running setup:**
```bash
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh \
  | SKRILLS_NO_HOOK=1 sh
```

## What the Installer Configures

### MCP Server Registration
The installer runs `skrills setup`, which registers Skrills as an MCP server so your AI assistant can use it directly:

- **Codex**: a `[mcp_servers.skrills]` table in `~/.codex/config.toml`. Setup does not write `~/.codex/mcp_servers.json`; `skrills doctor` checks `config.toml` and inspects a legacy `mcp_servers.json` only if one exists.
- **Claude Code**: `claude mcp add --scope user`, which records the server in `~/.claude.json`. If the `claude` command is missing or fails, setup writes `~/.claude/.mcp.json` instead.
- **Copilot**: `mcp_servers.json` in `~/.copilot`, or in the platform config directory (`~/.config/copilot` on Linux) when `~/.copilot` does not exist.
- **Cursor**: `~/.cursor/mcp.json`.

Set `SKRILLS_NO_HOOK=1` to skip this step.

### Hooks
Setup installs no hooks. `skrills setup --uninstall --client claude` removes the `~/.claude/hooks/prompt.on_user_prompt_submit` hook that older releases created.

### Skill Mirroring
The installer does not copy skills between clients. Set `SKRILLS_UNIVERSAL=1` to have setup also sync your Claude skill directories into `~/.agent/skills`. Use `skrills sync-all` to sync skills, commands and settings between clients.

## Troubleshooting

### "Command not found" after installation

Add the bin directory to your PATH:

```bash
export PATH="$HOME/.skrills/bin:$PATH"
```

Use the directory you set in `SKRILLS_BIN_DIR` if you changed it.

Add this line to your shell profile (`~/.bashrc`, `~/.zshrc`, etc.) to make it permanent.

### MCP server not recognized

Re-run the installer or manually register:

```bash
skrills setup --client codex --reinstall
```

Then run `skrills doctor` to verify.

### Wrong platform binary

If the installer picks the wrong architecture, specify it explicitly:

```bash
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh \
  | SKRILLS_TARGET=x86_64-unknown-linux-gnu sh
```

Find your target triple with:
```bash
rustc -vV | grep host
```

## Development Setup

For contributors, the Makefile provides common targets:

```bash
make build         # Release build
make test          # Run tests
make lint          # Run linting
make book          # Build this documentation
make book-serve    # Live preview on localhost:3000
```

## Next Steps

- Run `skrills validate` to check your skills
- See [CLI Usage Reference](cli.md) for all commands
- Check [Runtime Configuration](runtime-tuning.md) to customize behavior
