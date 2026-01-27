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

Skills support engine for Claude Code, Codex CLI, and GitHub Copilot CLI.

[Installation](book/src/installation.md) |
[User Guide](https://athola.github.io/skrills/) |
[CLI Reference](book/src/cli.md) |
[MCP Tutorial](docs/tutorials/mcp.md) |
[FAQ](docs/FAQ.md) |
[Changelog](book/src/changelog.md)

## Features

Skrills validates skills against Claude Code (permissive), Codex CLI (strict), and Copilot CLI (strict) rules. It syncs skills, commands, agents, MCP servers, and preferences across all three environments, preventing configuration drift. The validation engine derives missing YAML frontmatter from file paths and content to fix common errors automatically.

For optimization, Skrills analyzes token usage per skill and suggests reductions to fit context windows. It resolves skill dependencies with cycle detection and semantic versioning constraints. The built-in MCP server provides over 40 tools for validation, sync, and project-aware skill generation, while session mining improves recommendations based on actual usage.

## Demo

![Skrills Demo](assets/gifs/quickstart.gif)

See the [quickstart tutorial](docs/tutorials/quickstart.md) for a detailed walkthrough, or the [MCP tutorial](docs/tutorials/mcp.md) for server setup.

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

See [installation guide](book/src/installation.md) for HTTP transport setup, systemd services, and advanced options.

## Quickstart

```bash
# Validate skills for Codex/Copilot compatibility
skrills validate --target all --autofix

# Analyze token usage
skrills analyze --min-tokens 1000 --suggestions

# Sync from Claude to all other CLIs
skrills sync-all

# Start MCP server
skrills serve

# Interactive mode
skrills tui
```

See [CLI reference](book/src/cli.md) for all 38 commands including skill lifecycle management.

## Skill Management

Beyond validation and analysis, Skrills provides tools for managing skill lifecycles:

```bash
# Deprecate a skill with migration guidance
skrills skill-deprecate old-skill --replace "new-skill" --reason "Replaced by more efficient version"

# Rollback a skill to a previous git version
skrills skill-rollback my-skill --commit abc123

# Import skills from external sources
skrills skill-import https://example.com/skill.md
skrills skill-import ~/local/skills/

# Generate usage reports
skrills skill-usage-report --format json > report.json

# Calculate quality scores
skrills skill-score --min-score 80
```

## Why Skrills

Claude Code, Codex CLI, and Copilot CLI have different requirements for skill definitions. Codex and Copilot require YAML frontmatter with specific character limits (`name` max 100, `description` max 500), while Claude is permissive. Skrills catches these discrepancies at validation time, preventing runtime errors.

The sync system uses file hashing to respect manual edits, ensuring user changes aren't overwritten. Token analytics and dependency resolution help maintain a clean, efficient skill library within context limits.

## Limitations

- **No runtime skill injection**: Skrills validates and syncs files; it does not inject skills into prompts at runtime
- **Copilot command sync**: Copilot CLI does not support slash commands, so command sync is skipped
- **Empirical mining**: Session history parsing works best with recent Claude Code / Codex CLI versions
- **LLM generation**: Requires `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` for skill creation

## Architecture

| Crate | Purpose |
|-------|---------|
| `server` | MCP server, CLI, HTTP transport, security middleware |
| `validate` | Validation logic for Claude/Codex/Copilot compatibility |
| `analyze` | Token counting, dependency analysis, optimization |
| `intelligence` | Recommendations, project analysis, skill generation |
| `sync` | Multi-directional sync with adapters for each CLI |
| `discovery` | Skill discovery and ranking |
| `state` | Environment config, manifest settings, runtime overrides |
| `subagents` | Shared subagent runtime and backends |
| `test-utils` | Shared test infrastructure |

See [architecture docs](docs/architecture.md) for details.

## Configuration

Create `~/.skrills/config.toml` for persistent settings:

```toml
[serve]
auth_token = "your-secret-token"
tls_auto = true
cors_origins = "https://app.example.com"
```

Precedence: CLI flags > environment variables > config file.

See [security docs](docs/security.md) for TLS setup and [FAQ](docs/FAQ.md) for environment variables.

## Documentation

| Resource | Description |
|----------|-------------|
| [User Guide](https://athola.github.io/skrills/) | Primary documentation (mdBook) |
| [CLI Reference](book/src/cli.md) | All commands with examples |
| [MCP Tutorial](docs/tutorials/mcp.md) | Server setup and tool reference |
| [Sync Guide](book/src/sync-guide.md) | Cross-CLI sync workflows |
| [Token Optimization](book/src/mcp-token-optimization.md) | Context window management |
| [FAQ](docs/FAQ.md) | Common questions |
| [Security](docs/security.md) | Auth, TLS, threat model |
| [Changelog](book/src/changelog.md) | Release history |

## Development

```bash
make lint test --quiet
```

Requires Rust 1.75+. See [development guide](book/src/development.md) for test coverage, CI, and contribution workflow.

## Contributing

- **Security issues**: See [security policy](docs/security.md) and [threat model](docs/threat-model.md)
- **Bug reports**: Include OS, `skrills --version`, and logs (`--trace-wire` for MCP)
- **Pull requests**: Follow [process guidelines](docs/process-guidelines.md); update docs/tests with code

## License

[MIT](LICENSE)

[![Star History Chart](https://api.star-history.com/svg?repos=athola/skrills&type=date&legend=top-left)](https://www.star-history.com/#athola/skrills&type=date&legend=top-left)
