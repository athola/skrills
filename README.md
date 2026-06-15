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

Write a skill once, use it everywhere. Skrills validates and syncs
skills, commands, agents, MCP servers, and hooks across **Claude Code,
Codex CLI, GitHub Copilot CLI, and Cursor**, all from a single Rust binary.

[Install](book/src/installation.md) ·
[User Guide](https://athola.github.io/skrills/) ·
[CLI Reference](book/src/cli.md) ·
[FAQ](docs/FAQ.md) ·
[Changelog](book/src/changelog.md)

## Install

**macOS / Linux:**
```bash
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh
```

**Windows PowerShell:**
```powershell
powershell -ExecutionPolicy Bypass -NoLogo -NoProfile -Command ^
"Remove-Item alias:curl -ErrorAction SilentlyContinue; iwr https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.ps1 -UseBasicParsing | iex"
```

Or `cargo install skrills`. See the
[installation guide](book/src/installation.md) for HTTP transport and
service setup.

## Everyday use

Skrills handles the loop of writing skills and keeping every CLI
current. The common jobs:

**Make your Claude skills work everywhere.** Codex and Copilot are
strict about frontmatter; Cursor uses its own rule format. Detect and
fix the incompatibilities, then push to the other CLIs:

```bash
skrills validate --target both --autofix   # fix missing/invalid frontmatter and body
skrills sync-all                            # mirror everything to all four CLIs
```

**Keep two environments in sync.** Edit in one, mirror to another.
File hashing preserves manual edits on both sides:

```bash
skrills sync --from cursor --to claude
```

**Trim context-window cost.** Surface your token-heaviest skills with
reduction suggestions:

```bash
skrills analyze --min-tokens 1000 --suggestions
```

**Watch what's loaded, live.** `cold-window` continuously re-reads your
plugins, skills, commands, and subagents, applying per-source token
attribution and tiered alerts. Render it as a terminal TUI or a browser
dashboard (or both):

```bash
skrills cold-window --tui                    # live TUI in this terminal (q / Ctrl-C to quit)
skrills cold-window --browser --port 8888    # same engine, browser dashboard at /dashboard
```

**Keep it always-on in [Zellij](https://zellij.dev).** Dedicate a pane
to the TUI that respawns if it exits. Save as
`~/.config/zellij/layouts/skrills.kdl` and launch with
`zellij --layout skrills`:

```kdl
layout {
    tab name="cold-window" focus=true {
        pane command="bash" {
            // Loop respawns the TUI after a crash or reboot.
            args "-c" "until skrills cold-window --tui; do echo restarting...; sleep 2; done"
        }
    }
}
```

Already inside a Zellij session? Open it in a split without a layout
file: `zellij run -d down -- bash -c 'until skrills cold-window --tui; do sleep 2; done'`.

![Skrills cold-window TUI](assets/gifs/cold-window.gif)

**Let other tools call Skrills.** The MCP server exposes 36 tools
(validation, sync, intelligence, research) over stdio or HTTP:

```bash
skrills serve --http 127.0.0.1:3000 --open
```

See the [quickstart tutorial](docs/tutorials/quickstart.md) for a full
walkthrough and the [CLI reference](book/src/cli.md) for every command,
including skill lifecycle tools (`skill-deprecate`, `skill-rollback`,
`skill-import`, `skill-score`, `skill-catalog`).

![Skrills Demo](assets/gifs/quickstart.gif)

## Supported environments

Skrills syncs eight asset types across four CLIs. Each cell reflects
what the adapter reads and writes today:

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

A dash means that asset doesn't sync to that CLI, either because the CLI
has no equivalent (Copilot CLI has no slash commands) or because the
mapping isn't built yet (Cursor preferences). Plugin assets sync to
Cursor's `plugins/local/` so synced plugins appear installed, and
stale entries are pruned automatically. See the
[sync guide](book/src/sync-guide.md) for details.

## CI integration

Validate skills on every pull request with the reusable GitHub Action:

```yaml
- uses: athola/skrills/.github/actions/validate-skills@v0.8.2
  with:
    targets: all
    strict: true
    path: skills/
```

## Configuration

Persistent settings live in `~/.skrills/config.toml` (precedence: CLI
flags > environment variables > config file):

```toml
[serve]
auth_token = "your-secret-token"
tls_auto = true
cors_origins = "https://app.example.com"
```

See [security docs](docs/security.md) for TLS setup and the
[FAQ](docs/FAQ.md) for environment variables.

## Documentation

| Resource | Description |
|----------|-------------|
| [User Guide](https://athola.github.io/skrills/) | Primary documentation (mdBook) |
| [CLI Reference](book/src/cli.md) | All commands with examples |
| [Sync Guide](book/src/sync-guide.md) | Cross-CLI sync workflows |
| [MCP Tutorial](docs/tutorials/mcp.md) | Server setup and tool reference |
| [Architecture](docs/architecture.md) | Crate graph and runtime flow |
| [Security](docs/security.md) | Auth, TLS, threat model |
| [Changelog](book/src/changelog.md) | Release history |

## Limitations

- Skrills validates and syncs files; it does **not** inject skills into
  prompts at runtime.
- Session-history mining works best with recent Claude Code / Codex CLI
  versions.
- LLM-based skill generation requires `ANTHROPIC_API_KEY` or
  `OPENAI_API_KEY`.

## Contributing

```bash
make lint test --quiet
```

Builds on stable Rust. See the
[development guide](book/src/development.md) and
[process guidelines](docs/process-guidelines.md). Update docs and tests
with code. Report bugs with your OS, `skrills --version`, and logs
(`--trace-wire` for MCP). For security, see the
[security policy](docs/security.md).

## License

[MIT](LICENSE)

[![Star History Chart](https://api.star-history.com/svg?repos=athola/skrills&type=date&legend=top-left)](https://www.star-history.com/#athola/skrills&type=date&legend=top-left)
