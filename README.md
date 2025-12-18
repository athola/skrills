# Skrills

<p align="center">
  <img src="assets/icon.png" alt="Skrills Icon">
</p>

[![Crates.io](https://img.shields.io/crates/v/skrills.svg)](https://crates.io/crates/skrills)
[![Docs](https://img.shields.io/github/actions/workflow/status/athola/skrills/book-pages.yml?branch=master&label=docs)](https://athola.github.io/skrills/)
[![CI](https://img.shields.io/github/actions/workflow/status/athola/skrills/ci.yml?branch=master&label=ci)](https://github.com/athola/skrills/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/github/actions/workflow/status/athola/skrills/coverage.yml?branch=master&label=coverage)](https://github.com/athola/skrills/actions/workflows/coverage.yml)
[![Audit](https://img.shields.io/github/actions/workflow/status/athola/skrills/audit.yml?branch=master&label=audit)](https://github.com/athola/skrills/actions/workflows/audit.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Skills support engine for Claude Code and Codex CLI. Validates, analyzes, and syncs skills bidirectionally between both CLIs.

## Why Skrills
- **Validate skills**: Checks markdown against Claude Code (permissive) and Codex CLI (strict frontmatter) rules. Auto-fix adds missing metadata.
- **Analyze skills**: Reports token usage, identifies dependencies, and suggests optimizations.
- **Sync**: Bidirectional sync for skills, commands, MCP servers, and preferences between Claude Code and Codex CLI.
- **Safe command sync**: `sync-commands` uses byte-for-byte comparison and `--skip-existing-commands` to prevent overwriting local customizations. Preserves non-UTF-8 binaries.
- **Unified tools**: Mirror (`mirror`), sync (`sync`, `sync-all`), interactive diagnostics (`tui`), and agent launcher (`skrills agent <name>`) in one binary.

## Architecture (workspace crates)
- `crates/server`: MCP server runtime and CLI.
- `crates/validate`: Validation logic for Claude Code and Codex CLI compatibility.
- `crates/analyze`: Token counting, dependency analysis, and optimization.
- `crates/sync`: Bidirectional sync logic (skills, commands, prefs, MCP servers).
- `crates/discovery`: Skill discovery and ranking.
- `crates/state`: Persistent store for manifests and mirrors.
- `crates/subagents`: Shared subagent runtime and backends.

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
### Validate skills for Codex compatibility
```bash
skrills validate --target codex
skrills validate --target codex --autofix  # Auto-add missing frontmatter
```

### Analyze skill token usage
```bash
skrills analyze --min-tokens 1000 --suggestions
```

### Sync all configurations from Claude to Codex
```bash
skrills sync-all --from claude --skip-existing-commands
```

### Start MCP server
```bash
skrills serve
```

### Launch a mirrored agent spec
```bash
skrills agent codex-dev
```

## Validation

Skrills validates skills against two targets:

- **Claude Code**: Permissive. Accepts any markdown with optional frontmatter.
- **Codex CLI**: Strict. Requires YAML frontmatter with `name` (max 100 chars) and `description` (max 500 chars).

The `--autofix` flag derives missing frontmatter from the file path and content.

## MCP Tools

When running as an MCP server (`skrills serve`), the following tools are available:

- `validate-skills` - Validate skills for CLI compatibility
- `analyze-skills` - Analyze token usage and dependencies
- `skill-metrics` - Aggregate statistics (quality, tokens, dependencies)
- `sync-skills` - Sync skills between Claude and Codex
- `sync-commands` - Sync slash commands
- `sync-mcp-servers` - Sync MCP configurations
- `sync-preferences` - Sync preferences
- `sync-all` - Sync everything
- `sync-status` - Preview sync changes (dry run)
- `skill-loading-status` - Report skill roots, trace/probe install status, and marker coverage
- `enable-skill-trace` - Install trace/probe skills and optionally instrument SKILL.md files with markers
- `disable-skill-trace` - Remove trace/probe skill directories (does not remove markers)
- `skill-loading-selftest` - Return a one-shot probe line and expected response to confirm skills are loading

## CLI guide (selected)
- `skrills validate [--target claude|codex|both] [--autofix]` — validate skills for CLI compatibility.
- `skrills analyze [--min-tokens N] [--suggestions]` — analyze token usage and dependencies.
- `skrills metrics [--format text|json] [--include-validation]` — aggregate statistics and quality distribution.
- `skrills sync-all [--from claude|codex] [--skip-existing-commands]` — sync all configurations.
- `skrills sync-commands [--from claude|codex] [--dry-run] [--skip-existing-commands]` — byte-for-byte command sync.
- `skrills mirror` — mirror skills/agents/commands/prefs from Claude to Codex.
- `skrills tui` — interactive sync and diagnostics.
- `skrills doctor` — verify Codex MCP wiring.
- `skrills agent <name>` — launch a mirrored agent spec.

## Configuration
- `SKRILLS_MIRROR_SOURCE` — mirror source root (default `~/.claude`).
- `SKRILLS_CACHE_TTL_MS` — discovery cache TTL.
- `SKRILLS_CLIENT` — force installer target (`codex` or `claude`).
- `SKRILLS_NO_MIRROR=1` — skip post-install mirror on Codex.
- Subagents ship **on by default**: binaries are built with the `subagents` feature.
- `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` — default backend (`codex` or `claude`) when launching subagents.
- `~/.codex/subagents.toml` — optional override file for subagent defaults.

## Documentation
- mdBook (primary): https://athola.github.io/skrills/
  - `book/src/cli.md` — CLI reference.
  - `book/src/persistence.md` — state, pins, cache, mirrors (byte-safe command sync).
  - `book/src/overview.md` — discovery priority and architecture.
  - `book/src/mcp-token-optimization.md` — token-saving patterns.
- Additional docs in `docs/`:
  - `docs/architecture.md`, `docs/adr/` — architecture decisions and crate structure.
  - `docs/FAQ.md`, `docs/security.md`, `docs/threat-model.md`, `docs/semver-policy.md`, `docs/CHANGELOG.md`, `docs/process-guidelines.md`, `docs/dependencies.md`, `docs/release-artifacts.md`, `docs/config/`.
- Examples and demos: `examples/`, `crates/subagents/`, and TUI walkthroughs.

## Development
```bash
make fmt lint test --quiet
```
- Rust toolchain ≥ 1.75 recommended.
- End-to-end MCP tests are in `crates/server/tests/`; sample agents in `crates/subagents/`.

## Skill loading validation (Claude Code and Codex)

Neither Claude Code nor Codex CLI guarantees a built-in, user-visible report of which `SKILL.md` files were injected into the current prompt. Skrills provides an opt-in, deterministic workflow for validation:

This workflow is for debugging. The trace/probe skills add prompt overhead; remove them when finished.

1. Call `enable-skill-trace` (use `dry_run: true` to preview). This installs two debug skills and can instrument skill files by appending `<!-- skrills-skill-id: ... -->` markers (with optional backups).
2. Restart the Claude/Codex session if the client does not hot-reload skills.
3. Call `skill-loading-selftest` and send the returned `probe_line`. Expect `SKRILLS_PROBE_OK:<token>`.
4. With tracing enabled and markers present, each assistant response should end with:
   - `SKRILLS_SKILLS_LOADED: [...]`
   - `SKRILLS_SKILLS_USED: [...]`

Use `skill-loading-status` to check which roots were scanned and whether markers are present. Use `disable-skill-trace` to remove the debug skills when finished (it does not remove markers).

Example MCP inputs (tool arguments):

`enable-skill-trace`:
```json
{ "target": "codex", "instrument": true, "backup": true, "dry_run": false }
```

`skill-loading-selftest`:
```json
{ "target": "codex" }
```

For a longer walkthrough, see `book/src/cli.md`.

## Status & roadmap
- Changelogs: `docs/CHANGELOG.md` and `book/src/changelog.md`.

## Contributing & support
- Security: see `docs/security.md` and `docs/threat-model.md`; report issues via standard disclosure channels.
- Issues/PRs: include OS, `skrills --version`, and logs (use `--trace-wire` for MCP).
- Follow `docs/process-guidelines.md`; update docs/tests with code changes.

## License
[MIT](LICENSE)

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=athola/skrills&type=date&legend=top-left)](https://www.star-history.com/#athola/skrills&type=date&legend=top-left)
