# Skrills

<p align="center">
  <img src="assets/icon.png" alt="Skrills Icon">
</p>

[![Crates.io](https://img.shields.io/crates/v/skrills.svg)](https://crates.io/crates/skrills)
[![Downloads](https://img.shields.io/crates/d/skrills.svg)](https://crates.io/crates/skrills)
[![Docs](https://img.shields.io/github/actions/workflow/status/athola/skrills/book-pages.yml?branch=master&label=docs)](https://athola.github.io/skrills/)
[![CI](https://img.shields.io/github/actions/workflow/status/athola/skrills/ci.yml?branch=master&label=ci)](https://github.com/athola/skrills/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/github/actions/workflow/status/athola/skrills/coverage.yml?branch=master&label=coverage)](https://github.com/athola/skrills/actions/workflows/coverage.yml)
[![Audit](https://img.shields.io/github/actions/workflow/status/athola/skrills/audit.yml?branch=master&label=audit)](https://github.com/athola/skrills/actions/workflows/audit.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Mentioned in Awesome Codex CLI](https://awesome.re/mentioned-badge.svg)](https://github.com/RoggeOhta/awesome-codex-cli)

Skills support engine for Claude Code, Codex CLI, GitHub Copilot CLI,
and Cursor.

[Installation](book/src/installation.md) |
[User Guide](https://athola.github.io/skrills/) |
[CLI Reference](book/src/cli.md) |
[MCP Tutorial](docs/tutorials/mcp.md) |
[FAQ](docs/FAQ.md) |
[Changelog](book/src/changelog.md)

> **What's new in 0.7.7** -- Manifest-only plugin sync writes
> `.cursor-plugin/plugin.json` to `plugins/local/` instead of
> mirroring the full cache. Plugin-aware skill writing organizes
> synced skills under their source plugin. Validation cache
> enables offline `skrills validate`.
> See [changelog](book/src/changelog.md).

## Why Skrills?

Skills authored for one AI coding assistant rarely work in another.
Skrills validates, analyzes, and syncs skills across four CLI
environments from a single Rust binary:

- **One-command sync** -- `skrills sync-all` mirrors skills,
  commands, agents, MCP servers, hooks, rules, preferences, and
  plugin assets between Claude Code, Codex CLI, Copilot CLI,
  and Cursor.
- **Validation with autofix** -- detects missing frontmatter,
  incompatible fields, and body issues across each target's
  requirements, then fixes them with `--autofix`.
- **36-tool MCP server** -- exposes validation, sync,
  intelligence, and research operations over stdio or HTTP so
  other tools can call Skrills programmatically.

See [comparison](book/src/comparison.md) for how Skrills differs
from static skill bundles, rules repositories, and local sync
CLIs.

## Demo

![Skrills Demo](assets/gifs/quickstart.gif)

**TUI dashboard** -- navigate skills, activity, and metrics with
keyboard shortcuts:

![TUI Dashboard](assets/gifs/dashboard.gif)

See the [quickstart tutorial](docs/tutorials/quickstart.md) for a
walkthrough, or the [MCP tutorial](docs/tutorials/mcp.md) for server
setup.

## Features

- **Cross-CLI validation** -- validates against Claude Code
  (permissive), Codex CLI (strict), Copilot CLI (strict), and
  Cursor rules. Auto-derives missing YAML frontmatter.
- **Multi-directional sync** -- syncs eight asset types across
  four CLIs. File hashing preserves manual edits.
- **Token analytics** -- per-skill token counts with reduction
  suggestions for context-window management.
- **Dependency resolution** -- cycle detection and semver
  constraints across skill graphs.
- **MCP server** -- 36 tools over stdio or HTTP for validation,
  sync, intelligence, research, and skill generation.
- **Session mining** -- parses Claude Code and Codex CLI session
  history to improve recommendations.
- **Dashboards** -- TUI and browser UIs showing skills,
  validation status, usage metrics, and MCP server configs.
  The standalone [`skrills-portal.html`](skrills-portal.html)
  works offline without a running server.
- **Plugin asset sync** -- writes plugin manifests to Cursor's
  `plugins/local/` directory so synced plugins appear as
  installed. Skills with plugin origin are organized under
  their source plugin. Stale plugin entries are pruned
  automatically.
- **Validation cache** -- caches validation results in SQLite
  so `skrills validate` works offline with staleness indicators.
- **GitHub Action** -- reusable action for validating skills in
  pull requests with configurable targets and strictness.
- **Discovery deduplication** -- frontmatter identity matching
  consolidates duplicate skill installations.

## Installation

**macOS / Linux:**
```bash
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
```

**Windows PowerShell:**
```powershell
powershell -ExecutionPolicy Bypass -NoLogo -NoProfile -Command ^
"Remove-Item alias:curl -ErrorAction SilentlyContinue; iwr https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.ps1 -UseBasicParsing | iex"
```

**crates.io:** `cargo install skrills`

See [installation guide](book/src/installation.md) for HTTP transport
setup, systemd services, and advanced options.

## Quickstart

```bash
# Validate skills for Codex/Copilot/Cursor compatibility
skrills validate --target both --autofix

# Analyze token usage
skrills analyze --min-tokens 1000 --suggestions

# Sync from Claude to all other CLIs
skrills sync-all

# Sync between specific environments
skrills sync --from cursor --to claude

# Start MCP server and open the browser dashboard
skrills serve --http 127.0.0.1:3000 --open

# Interactive TUI dashboard
skrills tui

# Launch an agent with automatic backend routing (Claude -> Codex fallback)
skrills multi-cli-agent my-agent
```

See [CLI reference](book/src/cli.md) for all commands including
skill lifecycle management (`skill-deprecate`, `skill-rollback`,
`skill-import`, `skill-score`, `skill-catalog`).

### CI Integration

Validate skills in pull requests with the reusable GitHub Action:

```yaml
- uses: athola/skrills/.github/actions/validate-skills@v0.7.7
  with:
    targets: all
    strict: true
    path: skills/
```

## Supported Environments

Skrills syncs eight asset types across four CLI environments.
Each cell reflects what the adapter reads and writes today:

| Asset | Claude Code | Codex CLI | Copilot CLI | Cursor |
|-------|:-----------:|:---------:|:-----------:|:------:|
| Skills | Y | Y | Y | Y |
| Commands | Y | Y | -- | Y |
| Agents | Y | -- | Y | Y |
| MCP Servers | Y | Y | Y | Y |
| Hooks | Y | -- | -- | Y |
| Instructions / Rules | Y | -- | Y | Y |
| Preferences | Y | Y | Y | -- |
| Plugin Assets | Y | -- | -- | Y |

Plugin asset sync writes `.cursor-plugin/plugin.json` manifests to
`~/.cursor/plugins/local/` so Cursor recognizes synced plugins.
Cursor discovers actual plugin content from `~/.claude/plugins/cache/`
natively. Stale plugin entries are pruned automatically during sync.

Cursor rules (`.mdc` files) are mapped bidirectionally via mode
derivation (`alwaysApply`, glob-scoped, agent-requested).
See [ADR 0006](docs/adr/0006-cursor-rules-mapping.md) for the
mapping strategy and [sync guide](book/src/sync-guide.md) for
workflows.

## Architecture

| Crate | Purpose |
|-------|---------|
| `cli` | Binary entry point (delegates to `server` crate) |
| `server` | MCP server, HTTP transport, security middleware |
| `validate` | Validation logic for Claude/Codex/Copilot/Cursor compatibility |
| `analyze` | Token counting, dependency analysis, optimization |
| `intelligence` | Recommendations, project analysis, skill generation |
| `sync` | Multi-directional sync with adapters for each CLI (Claude, Codex, Copilot, Cursor) |
| `dashboard` | TUI and browser-based skill visualization |
| `discovery` | Skill discovery and ranking |
| `state` | Environment config, manifest settings, runtime overrides, validation cache, network detection |
| `metrics` | SQLite-based telemetry for invocations, validations, sync |
| `subagents` | Shared subagent runtime and backends |
| `tome` | Research API orchestration, caching, PDF serving |
| `test-utils` | Shared test infrastructure (fixtures, RAII guards, temp dirs) |

See [architecture docs](docs/architecture.md) for the crate
dependency graph and runtime flow.

## Configuration

Create `~/.skrills/config.toml` for persistent settings:

```toml
[serve]
auth_token = "your-secret-token"
tls_auto = true
cors_origins = "https://app.example.com"
```

Precedence: CLI flags > environment variables > config file.

See [security docs](docs/security.md) for TLS setup (`skrills cert`
subcommand) and [FAQ](docs/FAQ.md) for environment variables.

## Documentation

| Resource | Description |
|----------|-------------|
| [User Guide](https://athola.github.io/skrills/) | Primary documentation (mdBook) |
| [CLI Reference](book/src/cli.md) | All commands with examples |
| [MCP Tutorial](docs/tutorials/mcp.md) | Server setup and tool reference |
| [Sync Guide](book/src/sync-guide.md) | Cross-CLI sync workflows (Claude, Codex, Copilot, Cursor) |
| [Token Optimization](book/src/mcp-token-optimization.md) | Context window management |
| [FAQ](docs/FAQ.md) | Common questions |
| [Security](docs/security.md) | Auth, TLS, threat model |
| [Changelog](book/src/changelog.md) | Release history |

## Known Limitations

- **No runtime skill injection**: Skrills validates and syncs
  files; it does not inject skills into prompts at runtime.
- **Copilot command sync**: Copilot CLI does not support slash
  commands, so command sync is skipped.
- **Cursor preferences**: Cursor preferences are not yet mapped;
  preference sync is skipped for Cursor targets.
- **Empirical mining**: Session history parsing works best with
  recent Claude Code / Codex CLI versions.
- **LLM generation**: Requires `ANTHROPIC_API_KEY` or
  `OPENAI_API_KEY` for skill creation.

## Plugin Marketplaces

Skrills validates, analyzes, and syncs skills from these exemplar
plugin marketplaces:

- [superpowers](https://github.com/obra/superpowers) -- Opinionated
  skill pack for Claude Code covering TDD, code review, planning,
  debugging, and development workflows.
- [claude-night-market](https://github.com/athola/claude-night-market)
  -- Plugin marketplace for Claude Code with skills, agents, hooks,
  and commands across multiple domains.

## Development

```bash
make lint test --quiet
```

Requires Rust 1.75+. See [development guide](book/src/development.md)
for test coverage, CI, and contribution workflow.

## Contributing

- **Security issues**: See [security policy](docs/security.md) and
  [threat model](docs/threat-model.md)
- **Bug reports**: Include OS, `skrills --version`, and logs
  (`--trace-wire` for MCP)
- **Pull requests**: Follow
  [process guidelines](docs/process-guidelines.md); update docs/tests
  with code

## License

[MIT](LICENSE)

[![Star History Chart](https://api.star-history.com/svg?repos=athola/skrills&type=date&legend=top-left)](https://www.star-history.com/#athola/skrills&type=date&legend=top-left)
