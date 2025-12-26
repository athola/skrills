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
2. Installs it to `~/.codex/bin` (or detects your setup)
3. Registers skrills as an MCP server
4. Syncs your Claude skills to Codex (if both exist)

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
| `SKRILLS_CLIENT` | Target `codex` or `claude` | Auto-detected |
| `SKRILLS_BIN_DIR` | Where to install the binary | `~/.codex/bin` |
| `SKRILLS_VERSION` | Install a specific version | Latest |
| `SKRILLS_NO_MIRROR` | Skip syncing Claude skills | Disabled |

### Examples

**Install for Claude Code only:**
```bash
SKRILLS_CLIENT=claude SKRILLS_BIN_DIR="$HOME/.claude/bin" \
  curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
```

**Install a specific version:**
```bash
SKRILLS_VERSION=0.4.0 \
  curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
```

**Skip syncing Claude skills to Codex:**
```bash
SKRILLS_NO_MIRROR=1 \
  curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
```

## What the Installer Configures

### MCP Server Registration

The installer registers skrills as an MCP server so your AI assistant can use it directly. Configuration appears in:
- `~/.codex/mcp_servers.json` — Codex MCP registry
- `~/.codex/config.toml` — Codex configuration

### Hooks (Claude Code only)

For Claude Code, the installer creates a hook at `~/.claude/hooks/prompt.on_user_prompt_submit` to integrate skrills features.

### Skill Mirroring

By default, the installer copies your Claude skills to `~/.codex/skills/` so Codex can discover them. Skip this with `SKRILLS_NO_MIRROR=1`.

## Troubleshooting

### "Command not found" after installation

Add the bin directory to your PATH:

```bash
# For Codex (default)
export PATH="$HOME/.codex/bin:$PATH"

# For Claude
export PATH="$HOME/.claude/bin:$PATH"
```

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
SKRILLS_TARGET=x86_64-unknown-linux-gnu \
  curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
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
