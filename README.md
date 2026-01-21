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

Skills support engine for Claude Code, Codex CLI, and GitHub Copilot CLI. Validates, analyzes, and syncs skills bidirectionally between all three CLIs.

## Demos

### Quickstart
Validate, analyze, and sync skills between Claude Code and Codex.

![Skrills Quickstart](assets/gifs/quickstart.gif)

*See the [quickstart tutorial](docs/tutorials/quickstart.md) for a detailed walkthrough.*

### MCP Integration
Use skrills as an MCP server for dynamic skill loading in both Claude Code and Codex.

![Skrills MCP](assets/gifs/mcp.gif)

*See the [MCP tutorial](docs/tutorials/mcp.md) for setup instructions.*

## Why Skrills
Skrills manages skills and configurations for Claude Code, Codex CLI, and GitHub Copilot CLI. Each CLI has different requirements: Claude Code accepts raw markdown, while Codex and Copilot require strict YAML frontmatter. Skrills validates these files against each CLI's rules, preventing compatibility errors. It also syncs skills bidirectionally, provides diagnostics, and runs an MCP server from a single binary.

The `sync-commands` tool checks file hashes before writing to preserve local customizations. Analytics tools report token usage to suggest optimizations for context window limits.

## Architecture (workspace crates)
- `crates/server`: MCP server runtime, CLI, and HTTP transport with security middleware.
  - `mcp_gateway/`: Context-optimized tool loading with lazy schema retrieval (0.4.9+).
- `crates/validate`: Validation logic for Claude Code, Codex CLI, and Copilot CLI compatibility.
- `crates/analyze`: Token counting, dependency analysis, and optimization.
- `crates/intelligence`: Recommendations, project analysis, skill generation, and usage analytics persistence.
- `crates/sync`: Bidirectional sync logic (skills, commands, agents, prefs, MCP servers) with adapters for Claude, Codex, and Copilot.
- `crates/discovery`: Skill discovery and ranking.
- `crates/state`: Environment configuration, manifest settings, and runtime overrides.
- `crates/subagents`: Shared subagent runtime and backends (including `StateRunStore::load_from_disk` for reloading persisted runs).

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

### Sync all configurations between CLIs
```bash
# Claude to ALL other CLIs (Codex + Copilot) - no flags needed
skrills sync-all

# Claude to a specific CLI only
skrills sync-all --to codex --skip-existing-commands
```

> **Example plugins**: [claude-night-market](https://github.com/athola/claude-night-market) provides a collection of Claude Code plugins (skills, agents, commands, hooks) that can be synced to Codex using skrills.

### Start MCP server
```bash
skrills serve
```

### Launch a mirrored agent spec
```bash
skrills agent codex-dev
```

## Remote MCP Access (Experimental)

Skrills can expose its MCP server over HTTP for remote client access. HTTP transport is enabled by default in all release binaries.

```bash
# Start server on HTTP (instead of stdio)
skrills serve --http 0.0.0.0:3000
```

### Installation with HTTP Transport

To install with HTTP transport pre-configured (instead of stdio), use the `--http` flag:

```bash
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh -s -- --http 127.0.0.1:3000
```

This will:
- Install a systemd user service that runs `skrills serve --http <addr>`
- Configure MCP clients with the HTTP URL instead of stdio command
- Provide service management via `systemctl --user`

### Client Configuration

Configure your MCP client to connect:

```json
{
  "mcpServers": {
    "skrills-remote": {
      "url": "http://your-server:3000/mcp"
    }
  }
}
```

### Security Options (0.4.9+)

The `serve` command supports authentication and TLS for production deployments:

```bash
# Bearer token authentication
skrills serve --http 0.0.0.0:3000 --auth-token "your-secret-token"

# TLS encryption
skrills serve --http 0.0.0.0:3000 --tls-cert /path/to/cert.pem --tls-key /path/to/key.pem

# CORS for browser clients
skrills serve --http 0.0.0.0:3000 --cors-origins "http://localhost:3000,https://app.example.com"

# Full production setup (all options combined)
skrills serve --http 0.0.0.0:3000 \
  --auth-token "$SKRILLS_AUTH_TOKEN" \
  --tls-cert /etc/ssl/certs/skrills.pem \
  --tls-key /etc/ssl/private/skrills.key \
  --cors-origins "https://app.example.com"
```

The `--auth-token` flag also reads from `SKRILLS_AUTH_TOKEN` environment variable.

For deployments without these options, use SSH tunneling:

```bash
ssh -L 3000:localhost:3000 your-server
# Then connect to localhost:3000
```

## Validation

Skrills validates skills against three targets. **Claude Code** is permissive, accepting any markdown with optional frontmatter. **Codex CLI** and **GitHub Copilot CLI** are strict, requiring YAML frontmatter with a `name` (max 100 chars) and `description` (max 500 chars).

```bash
skrills validate --target codex      # Validate for Codex
skrills validate --target copilot    # Validate for Copilot
skrills validate --target all        # Validate for all CLIs
```

The `--autofix` flag derives missing frontmatter from the file path and content.

## MCP Tools

When running as an MCP server (`skrills serve`), several categories of tools become available.

**Validation & Analysis**
`validate-skills` checks CLI compatibility, while `analyze-skills` reports token usage and dependencies. `skill-metrics` aggregates quality and dependency statistics.

**Synchronization**
A suite of tools handles data transfer between Claude, Codex, and Copilot. `sync-skills`, `sync-commands`, `sync-mcp-servers`, `sync-preferences`, and `sync-all` cover various configuration aspects. `sync-from-claude`, `sync-from-copilot`, and `sync-to-copilot` handle direction-specific sync. `sync-status` provides a dry-run preview of changes. Agent sync from Claude to Copilot transforms the format automatically (removes `model`/`color`, adds `target: github-copilot`). Note: Copilot does not support slash commands, so command sync is skipped when Copilot is the source or target.

**Dependencies & Loading**
`resolve-dependencies` finds direct or transitive relationships for a skill URI. `skill-loading-status` reports on root scanning and marker coverage. `enable-skill-trace` and `disable-skill-trace` manage debug skills for tracing, while `skill-loading-selftest` confirms loading via a probe.

**Intelligence**
`recommend-skills` suggests related skills based on dependencies, and `recommend-skills-smart` adds usage patterns and project context to these recommendations. `analyze-project-context` scans languages and frameworks to inform suggestions. `suggest-new-skills` identifies gaps, and `create-skill` generates new skills via GitHub search, LLM generation, or empirical patterns. `search-skills-github` and `search-skills-fuzzy` provide search capabilities. Fuzzy search matches against both skill names and descriptions (0.4.8+).

**Context Optimization (0.4.9+)**
`list-mcp-tools` returns tool names and descriptions without full schemas. `describe-mcp-tool` loads the full schema for a specific tool. `get-context-stats` reports estimated tokens saved and schema load efficiency.

> **Testing**: Tool handler tests cover edge cases, dry-run modes, and target validation for Claude, Codex, Copilot, and all targets.

### Intelligence tools
- `recommend-skills-smart` - Smart recommendations using dependencies, usage patterns, and project context
- `analyze-project-context` - Analyze languages, frameworks, and keywords in a project directory
- `suggest-new-skills` - Identify skill gaps based on context and usage
- `create-skill` - Create a new skill via GitHub search, LLM generation, empirical patterns, or combinations
- `search-skills-github` - Search GitHub for existing `SKILL.md` files
- `search-skills-fuzzy` - Trigram-based fuzzy search for installed skills (typo-tolerant, matches names and descriptions)

#### Empirical skill creation (0.4.4+)
The `--method empirical` option mines Claude Code and Codex CLI session history to extract successful tool sequences and failure anti-patterns. It clusters similar sessions and generates skills grounded in observed behavior rather than LLM imagination.

#### Comparative recommendations (0.4.4+)
Deviation scoring compares actual skill-assisted outcomes against category baselines (Testing, Debugging, Documentation, etc.) to identify underperforming skills and surface improvement opportunities.

**CLI parity notes**:
- `skrills sync-from-claude` is an alias for `skrills sync` (copy Claude skills into the Codex mirror).
- `sync-from-copilot` and `sync-to-copilot` are MCP-only tools. Use `skrills sync-all --from copilot` or `skrills sync-all --to copilot` for CLI equivalents.
- `resolve-dependencies` and the intelligence tools are available via CLI commands (see below).

### MCP tool inputs (selected)
`sync-from-claude`:
```json
{}
```

`resolve-dependencies`:
```json
{ "uri": "skill://skrills/codex/my-skill/SKILL.md", "direction": "dependencies", "transitive": true }
```

`search-skills-fuzzy`:
```json
{ "query": "databas", "threshold": 0.3, "limit": 10 }
```

### Smart recommendation workflows (examples)
1. Project-aware recommendations:
   - `analyze-project-context` -> `recommend-skills-smart` -> `suggest-new-skills`
2. GitHub-assisted skill creation:
   - `search-skills-github` -> `create-skill` (use `dry_run: true` to preview)
3. Fuzzy skill discovery (typo-tolerant):
   - `search-skills-fuzzy` with query `"databas"` finds `"database"` skills

## CLI guide (selected)
- `skrills validate [--target claude|codex|copilot|both|all] [--autofix]` — validate skills for CLI compatibility.
- `skrills analyze [--min-tokens N] [--suggestions]` — analyze token usage and dependencies.
- `skrills metrics [--format text|json] [--include-validation]` — aggregate statistics and quality distribution.
- `skrills recommend <uri> [--limit N] [--include-quality]` — suggest related skills based on dependencies.
- `skrills resolve-dependencies <uri> [--direction dependencies|dependents] [--transitive] [--format text|json]` — resolve dependencies or dependents.
- `skrills recommend-skills-smart [--uri URI] [--prompt TEXT] [--project-dir DIR] [--auto-persist]` — smart recommendations using usage and context (auto-persist saves analytics to cache).
- `skrills analyze-project-context [--project-dir DIR] [--include-git true|false] [--commit-limit N] [--format text|json]` — analyze project context for recommendations.
- `skrills suggest-new-skills [--project-dir DIR] [--focus-area AREA]` — identify gaps and suggestions.
- `skrills create-skill <name> --description TEXT [--method github|llm|empirical|both] [--target-dir DIR]` — create skills via GitHub search, LLM generation, or empirical session patterns (target dir defaults to installed client, Claude preferred).
- `skrills search-skills-github <query> [--limit N] [--format text|json]` — search GitHub for skills.
- `skrills sync-all [--from claude|codex|copilot] [--to claude|codex|copilot] [--skip-existing-commands]` — sync all configurations.
- `skrills sync-commands [--from claude|codex] [--dry-run] [--skip-existing-commands]` — byte-for-byte command sync (Copilot does not support commands).
- `skrills mirror` — mirror skills/agents/commands/prefs from Claude to Codex.
- `skrills tui` — interactive sync and diagnostics.
- `skrills doctor` — verify Codex MCP wiring.
- `skrills agent <name>` — launch a mirrored agent spec.
- `skrills export-analytics [--output PATH] [--force-rebuild]` — export usage analytics to file.
- `skrills import-analytics <input> [--overwrite]` — import analytics from exported file.

## Configuration
- `SKRILLS_MIRROR_SOURCE` — mirror source root (default `~/.claude`).
- `SKRILLS_CACHE_TTL_MS` — discovery cache TTL.
- `SKRILLS_CLIENT` — force installer target (`codex` or `claude`).
- `SKRILLS_AUTO_PERSIST` — auto-save analytics to cache after operations that build analytics (`1` or `true`).
- `SKRILLS_NO_MIRROR=1` — skip post-install mirror on Codex.
- `GITHUB_TOKEN` — optional GitHub API token for `search-skills-github` and GitHub-backed `create-skill` to avoid rate limits.
- Subagents ship **on by default**: binaries are built with the `subagents` feature.
- `SKRILLS_SUBAGENTS_EXECUTION_MODE` — default subagent mode (`cli` or `api`, default `cli`).
- `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` — default API backend (`codex` or `claude`) when `execution_mode=api`.
- `SKRILLS_CLI_BINARY` — override CLI binary for headless subagents (auto uses current client; default `claude`).
- `~/.claude/subagents.toml` or `~/.codex/subagents.toml` — optional defaults (`execution_mode`, `cli_binary`, `default_backend`; `cli_binary = "auto"` follows current client using CLI env or server path).

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
- Tool handler tests cover edge cases, dry-run modes, and target validation for all MCP tools (see `crates/server/src/app/tests.rs`).
- BDD-style unit tests cover schema generation (`crates/subagents/src/tool_schemas.rs`), sync reporting (`crates/sync/src/report.rs`), and validation issues (`crates/validate/src/common.rs`).

## Skill loading validation (Claude Code and Codex)

Claude Code and Codex CLI do not inherently report which `SKILL.md` files were injected into the current prompt. Skrills provides an opt-in, deterministic workflow for validation:

This workflow helps with debugging. The trace/probe skills add prompt overhead; remove them when finished.

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
