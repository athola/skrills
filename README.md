# Skrills

[![Crates.io](https://img.shields.io/crates/v/skrills.svg)](https://crates.io/crates/skrills)
[![Docs](https://img.shields.io/github/actions/workflow/status/athola/skrills/book-pages.yml?branch=master&label=docs)](https://athola.github.io/skrills/)
[![CI](https://img.shields.io/github/actions/workflow/status/athola/skrills/ci.yml?branch=master&label=ci)](https://github.com/athola/skrills/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/github/actions/workflow/status/athola/skrills/coverage.yml?branch=master&label=coverage)](https://github.com/athola/skrills/actions/workflows/coverage.yml)
[![Audit](https://img.shields.io/github/actions/workflow/status/athola/skrills/audit.yml?branch=master&label=audit)](https://github.com/athola/skrills/actions/workflows/audit.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Local MCP server that mirrors and auto-discovers skills, agents, commands, and preferences across Codex and Claude. Makes transferring your Claude Code experience over to Codex as seamless as possible. Designed for byte-safe mirroring, prompt-aware autoload, and transparent diagnostics.

## Why skrills
- Discovers skills across Codex, Claude, cache, mirrors, and agent roots with priority-based matching and TTL controls.
- Prompt-aware autoload using MCP server with pins, history, and byte budgets (manifest-first rendering).
- Byte-for-byte command sync with `--skip-existing-commands` to avoid overwriting local commands; non-UTF-8 commands are preserved.
- Full mirror + sync CLI/TUI (`mirror`, `sync`, `sync-all`, `tui`) and agent launcher (`skrills agent <name>`) so you can easily sync your skills from Claude and vice-versa.
- Rich diagnostics: `render-preview`, `emit-autoload`, `runtime-status`, and TUI insights.

## Architecture (workspace crates)
- `crates/server`: MCP server runtime.
- `crates/discovery`: skill discovery, ranking, and autoload.
- `crates/sync`: mirroring between Claude/Codex (skills, commands, prefs, MCP servers).
- `crates/cli`: CLI entrypoints and TUI.
- `crates/state`: persistent store for pins, history, manifests, and mirrors.
- `crates/subagents`: shared subagent runtime and backends for Codex/Claude delegation (reused by CLI/TUI).

## Installation
- macOS / Linux:
  ```bash
  curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
  ```
- Windows PowerShell:
  ```powershell
  powershell -ExecutionPolicy Bypass -NoLogo -NoProfile -Command ^
  "Remove-Item alias:curl -ErrorAction SilentlyContinue; iwr https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.ps1 -UseBasicParsing | iex"
  ```
- crates.io: `cargo install skrills`
- From source: `cargo install --path crates/cli --force`

## Quickstart
```bash
# Start MCP server over stdio
skrills serve

# Mirror Claude assets into Codex (respects SKRILLS_MIRROR_SOURCE)
skrills sync-all --skip-existing-commands

# Preview autoload for a prompt
skrills render-preview --prompt "List debugging skills for MCP servers"

# Launch a mirrored agent spec
skrills agent codex-dev

# Run subagents (requires --features subagents)
skrills serve --features subagents
# Then from MCP client: list_subagents, run_subagent, get_run_status
```

## Autoload flow

```mermaid
flowchart LR
    U[User prompt]
    subgraph Client
      CX[Codex CLI / IDE]
      CL[Claude Code]
    end
    S[skrills MCP server]
    D[discover_skills<br/>cache snapshot + TTL]
    F[filter pins/history/preload<br/>+ prompt similarity]
    R[render bundle<br/>(manifest+content or manifest-only/gzip)]
    P[Append bundle to model prompt]

    U --> CX
    U --> CL
    CX -->|MCP tool: autoload-snippet(prompt)| S
    CL -->|MCP: listResources + readResource| S
    S --> D --> F --> R --> P
```

- Snapshot cache (`~/.codex/skills-cache.json` or `SKRILLS_CACHE_PATH`) is
  reloaded on invalidation/first access before scanning; if a scan returns no
  skills we keep the snapshot so snapshot-only skills remain available.
- Rendering respects byte budgets: manifest-first with gzip fallback when
  needed.
- See `book/src/prompt-loading.md` and `docs/prompt-skill-loading.md` for the
  full path and tuning flags.

## CLI guide (selected)
- `skrills mirror | sync | sync-all [--skip-existing-commands]` — mirror skills/agents/commands/prefs without overwriting existing commands.
- `skrills sync-commands [--from claude|codex] [--dry-run] [--skip-existing-commands]` — byte-for-byte command sync.
- `skrills sync import | export | report` — cross-agent skill synchronization with Claude/Codex adapters.
- `skrills render-preview` — show autoloaded skills for a prompt (no injection).
- `skrills list` — list discovered skills with source priority.
- `skrills pin | unpin` — manage pinned skills.
- `skrills tui` — interactive pinning, mirroring, diagnostics.
- `skrills emit-autoload` — emit the hook payload used by IDEs.
- `skrills doctor` — verify Codex MCP wiring.

## Configuration
- `SKRILLS_MIRROR_SOURCE` — mirror source root (default `~/.claude`).
- `SKRILLS_PINNED` — comma-separated skills pinned at startup.
- `SKRILLS_CACHE_TTL_MS` — discovery cache TTL.
- `SKRILLS_CLIENT` — force installer target (`codex` or `claude`).
- `SKRILLS_NO_MIRROR=1` — skip post-install mirror on Codex.
- Subagents ship **on by default**: binaries are built with the `subagents` feature and `scripts/install.sh` drops a default `subagents.toml` into your client root on first install.
- `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` — default backend (`codex` or `claude`) when launching subagents without an explicit backend.
- `~/.codex/subagents.toml` — optional override file for subagent defaults (see `docs/config/subagents.example.toml`).
- Manifest overrides: `~/.codex/skrills.manifest.json` (or client root). See `docs/runtime-options.md`.

## Documentation
- mdBook (primary): https://athola.github.io/skrills/
  - `book/src/cli.md` — CLI reference.
  - `book/src/autoload.md` — autoload + truncation rules.
  - `book/src/persistence.md` — state, pins, cache, mirrors (byte-safe command sync).
  - `book/src/overview.md` — discovery priority and architecture.
  - `book/src/mcp-token-optimization.md` — token-saving patterns.
- Additional docs in `docs/`:
  - `docs/FAQ.md`, `docs/runtime-options.md`, `docs/security.md`, `docs/threat-model.md`, `docs/semver-policy.md`, `docs/CHANGELOG.md`, `docs/process-guidelines.md`, `docs/dependencies.md`, `docs/release-artifacts.md`, `docs/config/`.
- Examples and demos: `examples/`, `crates/subagents/`, and TUI walkthroughs.

## Development
```bash
make format && make lint && make test --quiet && make build
```
- Rust toolchain ≥ 1.75 recommended.
- End-to-end MCP tests live under `crates/server/tests/`; sample agents in `crates/subagents/`.

## Status & roadmap
- Actively developed; changelogs in `docs/CHANGELOG.md` and `book/src/changelog.md`.

## Contributing & support
- Security: see `docs/security.md` and `docs/threat-model.md`; report issues via standard disclosure channels.
- Issues/PRs: include OS, `skrills --version`, and logs (use `--trace-wire` for MCP).
- Follow `docs/process-guidelines.md`; update docs/tests with code changes.

## License
[MIT](LICENSE)
