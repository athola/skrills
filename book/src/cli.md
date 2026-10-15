# CLI Reference

Commands organized by task.

## Checking Your Skills

### validate

Verify skills work with Claude Code, Codex CLI, or both:

```bash
skrills validate                              # Check all skills for both platforms
skrills validate --target codex               # Check Codex compatibility only
skrills validate --target codex --autofix     # Auto-fix missing frontmatter
skrills validate --format json --errors-only  # Machine-readable output
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--skill-dir <DIR>` | Check a specific directory (default: all discovered skills) |
| `--target <TARGET>` | `claude`, `codex`, `copilot`, `all`, or `both` (Claude and Codex; default: `both`) |
| `--autofix` | Add missing frontmatter automatically |
| `--backup` | Create backups before autofix |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |
| `--errors-only` | Hide passing skills |

`validate` exits non-zero when any skill has an error, cannot be read, or
could not be auto-fixed, so a script or CI step can gate on the exit code.
`--watch` is not implemented yet and is refused.

### analyze

Find skills that consume too many tokens or need optimization:

```bash
skrills analyze                               # Analyze all skills
skrills analyze --min-tokens 1000             # Show only large skills
skrills analyze --suggestions                 # Get optimization tips
skrills analyze --format json                 # Machine-readable output
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--skill-dir <DIR>` | Analyze a specific directory (default: all discovered skills) |
| `--min-tokens <N>` | Filter to skills exceeding this count |
| `--suggestions` | Include optimization recommendations |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |

### metrics

Get aggregate statistics about your skill collection:

```bash
skrills metrics                               # Summary statistics
skrills metrics --format json                 # Machine-readable output
skrills metrics --include-validation          # Include pass/fail counts (slower)
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--skill-dir <DIR>` | Include a specific directory (default: all discovered skills) |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |
| `--include-validation` | Include validation pass/fail counts |

**Output includes:**
- Skill counts by source (claude, codex, marketplace)
- Quality distribution (high/medium/low based on quality scores)
- Dependency statistics (total edges, orphan count, hub skills)
- Token usage (total, average, largest skill)

## Syncing Between Claude and Codex

### sync-all

Sync everything between Claude Code and Codex CLI:

```bash
skrills sync-all --from claude                        # Copy everything from Claude
skrills sync-all --from claude --skip-existing-commands  # Keep local commands
skrills sync-all --dry-run                            # Preview without changing
skrills sync-all --from claude --validate             # Refuse to sync skills that do not validate
skrills sync-all --from claude --autofix --dry-run    # Check what autofix would leave, write nothing
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--from` | Source CLI: `claude`, `codex`, `copilot`, or `cursor` (default: `claude`) |
| `--to` | Target CLI, same values; omitted, syncs to every other CLI |
| `--dry-run` | Preview changes without writing |
| `--skip-existing-commands` | Keep existing commands on target side |
| `--include-marketplace` | Include marketplace content from uninstalled plugins (`SKRILLS_INCLUDE_MARKETPLACE`) |
| `--exclude-plugins <NAMES>` | Comma-separated plugin names whose skills and assets are not synced (`SKRILLS_EXCLUDE_PLUGINS`) |
| `--validate` | Validate the source skills before any target is written |
| `--autofix` | Add missing frontmatter to the source skills, then validate as `--validate` does |

With `--validate` or `--autofix`, the `--from` skill tree is validated for
the strictest of the targets being synced (Codex and Copilot are stricter
than Claude and Cursor) before the first target is touched. Any remaining
error aborts every target, with nothing written. `--autofix` rewrites the
source skills in place, without a backup, before validating; under
`--dry-run` it only checks what the fix would leave and rewrites nothing.

### sync-status

Preview what would change before syncing:

```bash
skrills sync-status --from claude
```

### Individual Sync Commands

Sync specific items when you don't want everything:

```bash
skrills sync                          # Skills only (alias: sync-from-claude)
skrills sync-commands --from claude
skrills sync-mcp-servers --from claude
skrills sync-preferences --from claude
```

**Technical notes:**
- `skrills sync` honors `SKRILLS_MIRROR_SOURCE` to change the source root
- Commands are copied byte-for-byte, preserving non-UTF-8 files without re-encoding
- Use `--skip-existing-commands` to preserve local command customizations

### mirror

Copy Claude assets to Codex defaults and refresh the agent list in
`~/.codex/AGENTS.md`:

```bash
skrills mirror                           # Full mirror
skrills mirror --skip-existing-commands  # Preserve existing commands
skrills mirror --dry-run                 # Preview changes (hashes sources, reports intended writes)
```

## Running the MCP Server

### serve

Start skrills as an MCP server (used by Claude Code, Codex CLI, Copilot CLI, and Cursor):

```bash
skrills serve                             # MCP server over stdio
skrills serve --http 127.0.0.1:3000 --open      # Browser dashboard with auto-launch
skrills serve --http 127.0.0.1:8080             # HTTP on a specific address and port
skrills serve --http 127.0.0.1:3000 --tls-auto  # HTTPS with auto-generated cert
skrills serve --watch                     # Auto-reload on file changes
skrills serve --cache-ttl-ms 300000       # 5-minute cache
skrills serve --skill-dir ~/.custom/skills  # Custom skill directory
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--http <BIND_ADDR>` | Serve MCP over HTTP, with the REST API and browser dashboard, on this address (for example `127.0.0.1:3000`); there is no separate port flag |
| `--open` | Auto-launch the browser dashboard after starting (requires `--http`) |
| `--tls-auto` | Generate self-signed TLS cert for HTTPS in development |
| `--allowed-hosts <HOSTS>` | Extra `Host` values the MCP transport accepts, on top of `localhost`, `127.0.0.1` and `::1` |
| `--skill-dir <DIR>` | Additional skill directory to include |
| `--cache-ttl-ms <N>` | Discovery cache TTL in milliseconds |
| `--watch` | Enable live filesystem invalidation (stdio only; refused with `--http`, so restart an HTTP server to pick up skill changes) |
| `--auth-token <TOKEN>` | Require `Authorization: Bearer <TOKEN>` on HTTP requests (`SKRILLS_AUTH_TOKEN`); unset, no auth |
| `--tls-cert <PATH>` / `--tls-key <PATH>` | Serve HTTPS with this PEM certificate and key; each requires the other (`SKRILLS_TLS_CERT`, `SKRILLS_TLS_KEY`) |
| `--cors-origins <ORIGINS>` | Comma-separated allowed CORS origins (`SKRILLS_CORS_ORIGINS`); unset, no CORS |
| `--list-tools` | Print the available MCP tools and exit |
| `--trace-wire` | Dump raw MCP initialize traffic as hex and UTF-8 for debugging (`SKRILLS_TRACE_WIRE`) |

With `--auth-token`, the dashboard page and its static files load without
the token, but every API call needs it. Open the dashboard as
`http://127.0.0.1:3000/#token=<TOKEN>` (the fragment never leaves the
browser), or enter the token when the page prompts after its first
rejected request.

The MCP transport validates the inbound `Host` header and accepts only
`localhost`, `127.0.0.1` and `::1` unless told otherwise. That is what stops a
page on an attacker's domain from resolving to your loopback address and
driving the server through your browser. Binding a non-loopback address needs
the hostname clients actually use:

```bash
skrills serve --http 0.0.0.0:8080 --allowed-hosts skrills.internal:8080
```

Supplied hosts are added to the loopback set, never substituted for it.
The same list is settable in the config file as `allowed_hosts` under
`[serve]`, or through `SKRILLS_ALLOWED_HOSTS`.

`recommend-skills-smart`, `analyze-project-context` and `suggest-new-skills`
take a `project_dir` from the client and read that directory, and
`create-skill` reads one when passed. To limit which directories they may read, list the allowed
roots in `~/.skrills/config.toml`:

```toml
[serve]
project_roots = ["~/src", "/work"]
```

A `project_dir` outside every root is refused with an `invalid_params` error
naming the setting. Both sides are canonicalized before the check, so `..`
segments and symlinks are judged by where they lead, and a `project_dir` that
does not exist is refused too. `~` expands to your home directory. When
`project_roots` is unset, any directory is accepted, and `serve --http` on a
non-loopback address logs a warning at startup. The check applies only to a
`project_dir` the client passes; when it passes none, the tools use the
server's working directory.

The MCP server exposes 36 core tools (48 with the default subagent and gateway features) for validation, analysis, sync, intelligence, and research directly to your AI assistant. The HTTP mode serves a browser dashboard with skills explorer, metrics, and activity feed.

Skrills can generate a self-contained HTML portal (`skrills-portal.html`) via the `html-portal-generator` skill. It works offline without a running server and includes a skills browser, validator with autofix, token analyzer, cross-CLI converter, and full CLI/MCP reference. Open it directly in any browser or upload it into AI application portals. The file is git-ignored, so regenerate it as part of release prep.

## Working with Subagents

### agent

Run a discovered agent by name with Codex:

```bash
skrills agent codex-dev                   # Run the codex-dev agent
skrills agent my-agent --dry-run          # Preview without running
skrills agent my-agent --skill-dir ~/.custom/skills  # Include custom skills
```

The agent runs as `codex exec --sandbox workspace-write "<prompt>"`: it can
write only inside the current directory, and Codex refuses to start outside
a git repository or trusted directory. Agent files can come from third-party
plugin caches, so the sandbox stays on unless you pass
`--dangerously-bypass-sandbox`, which runs Codex with
`--dangerously-bypass-approvals-and-sandbox` (no sandbox, no approvals). Use
it only for agent files you trust, in an environment that is sandboxed
already.

| Option | Purpose |
|--------|---------|
| `--skill-dir <DIR>` | Additional agent directory (repeatable) |
| `--dry-run` | Print the resolved command without running it |
| `--dangerously-bypass-sandbox` | Run Codex with no sandbox and no approvals |
| `--timeout <SECONDS>` | Kill the agent after this many seconds (default: 1800; `0` = no limit) |

**Execution mode configuration (`run-subagent` MCP tool):**

The settings below apply to subagents run through the MCP server, not to
`skrills agent`. Subagent behavior comes from multiple sources (in priority order):
1. `SKRILLS_SUBAGENTS_EXECUTION_MODE` environment variable
2. `execution_mode` in `~/.claude/subagents.toml` or `~/.codex/subagents.toml`
3. Default: `cli`

When `execution_mode=api` and no backend is specified, skrills checks:
1. `default_backend` in the config file
2. `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` environment variable
3. Default: `codex`

In `cli` mode, when the run names no backend, skrills picks the binary in this order:
1. `cli_binary` (or `SKRILLS_CLI_BINARY`) when it names a binary rather than `auto`
2. The configured default backend (`SKRILLS_SUBAGENTS_DEFAULT_BACKEND` or `default_backend`): `codex` or `claude`
3. The current client, detected from `SKRILLS_CLIENT` or the CLI environment, then the server binary path
4. Default: `claude`

### multi-cli-agent

Launch an agent through Claude Code or Codex, falling back to the other
when the first choice is not installed:

```bash
skrills multi-cli-agent my-agent                    # Auto-detect the backend
skrills multi-cli-agent my-agent --backend codex    # Prefer Codex
skrills multi-cli-agent my-agent --dry-run          # Print the resolved command only
```

| Option | Purpose |
|--------|---------|
| `--backend <BACKEND>` | `auto`, `claude`, or `codex` (default: `auto`) |
| `--skill-dir <DIR>` | Additional skill directory (repeatable) |
| `--dry-run` | Print the resolved command without running it |
| `--dangerously-bypass-sandbox` | Codex backend only: run with no sandbox and no approvals (refused when Claude is selected) |
| `--timeout <SECONDS>` | Kill the agent after this many seconds (default: 1800; `0` = no limit) |

Codex runs with the same sandboxed command as `skrills agent`; Claude runs
as `claude --print "<prompt>"`. On timeout the command kills the CLI it started
(not processes that CLI spawned) and exits with an error naming the limit.

### sync-agents

Update the available skills list for agents:

```bash
skrills sync-agents                       # Refresh AGENTS.md
skrills sync-agents --path custom.md      # Write to different file
```

**Skill naming caveat:**

Skills are named from the `name:` field in `SKILL.md` frontmatter. Treat these names as opaque strings. They may include punctuation like `:` for namespacing (e.g., `pensive:shared`).

When comparing skills between session headers and disk, don't parse by splitting on `:`. Instead, extract the `(file: …/SKILL.md)` path or read the frontmatter directly.

## Finding and Creating Skills

### recommend

Discover skills related to one you use:

```bash
skrills recommend skill://skrills/codex/my-skill/SKILL.md
skrills recommend skill://skrills/codex/my-skill/SKILL.md --limit 5
skrills recommend skill://skrills/codex/my-skill/SKILL.md --format json
```

**Returns three types of recommendations:**
- **Dependencies**: Skills the target skill directly uses (highest priority)
- **Dependents**: Skills that use the target skill
- **Siblings**: Skills sharing common dependencies with the target

**Options:**

| Option | Purpose |
|--------|---------|
| `--skill-dir <DIR>` | Include a specific directory |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |
| `--limit <N>` | Maximum recommendations (default: 10) |
| `--include-quality` | Include quality scores (default: true) |

### recommend-skills-smart

Get context-aware recommendations combining dependencies, usage patterns, and project context:

```bash
skrills recommend-skills-smart --project-dir .
skrills recommend-skills-smart --uri skill://skrills/codex/my-skill/SKILL.md
skrills recommend-skills-smart --prompt "deployment pipeline" --include-usage false
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--uri <URI>` | Skill URI for dependency-based recommendations |
| `--prompt <TEXT>` | Text for semantic matching |
| `--project-dir <DIR>` | Project directory for context analysis |
| `--limit <N>` | Maximum recommendations (default: 10) |
| `--include-usage` | Include usage pattern analysis (default: true) |
| `--include-context` | Include project context analysis (default: true) |
| `--auto-persist` | Save analytics to cache after building |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |

### suggest-new-skills

Identify skill gaps in your collection:

```bash
skrills suggest-new-skills --project-dir .
skrills suggest-new-skills --focus-area testing
skrills suggest-new-skills --focus-area ci --focus-area deployment
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--project-dir <DIR>` | Project directory for context (default: cwd) |
| `--focus-area <AREA>` | Focus area to prioritize (repeatable) |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |

### create-skill

Generate a new skill via GitHub search, LLM generation, empirical patterns, or both:

```bash
skrills create-skill my-new-skill --description "Helps with testing"
skrills create-skill my-new-skill --description "..." --method github
skrills create-skill my-new-skill --description "..." --method empirical  # From session patterns
skrills create-skill my-new-skill --description "..." --dry-run  # Preview
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--description <TEXT>` | Required description of the skill |
| `--method <METHOD>` | `github`, `llm`, `empirical`, or `both` (default: `both`) |
| `--target-dir <DIR>` | Where to save (default: installed client, Claude preferred) |
| `--project-dir <DIR>` | Project directory for context |
| `--dry-run` | Preview without writing files |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |

The `empirical` method analyzes Claude Code and Codex CLI session history to extract successful tool sequences and failure anti-patterns. It clusters similar sessions and generates skills based on observed behavior.

### search-skills-github

Find existing skills on GitHub:

```bash
skrills search-skills-github "testing workflow"
skrills search-skills-github "deployment" --limit 20
```

### resolve-dependencies

Trace skill relationships:

```bash
skrills resolve-dependencies skill://skrills/codex/my-skill/SKILL.md
skrills resolve-dependencies skill://... --direction dependents
skrills resolve-dependencies skill://... --transitive false
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--direction` | `dependencies` or `dependents` (default: `dependencies`) |
| `--transitive` | Include transitive relationships (default: true) |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |

### analyze-project-context

Extract project characteristics for recommendations:

```bash
skrills analyze-project-context --project-dir .
skrills analyze-project-context --include-git true --commit-limit 100
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--project-dir <DIR>` | Project to analyze (default: cwd) |
| `--include-git` | Include git commit keyword analysis (default: true) |
| `--commit-limit <N>` | Number of commits to scan (default: 50) |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |

Returns languages, frameworks, and keywords detected in your project.

### search-skills

Fuzzy-search installed skills by name, and optionally by description:

```bash
skrills search-skills commit
skrills search-skills review --include-description --threshold 0.2 --limit 5
```

| Option | Purpose |
|--------|---------|
| `-t`, `--threshold <N>` | Similarity threshold from 0.0 to 1.0; lower returns more (default: `0.3`) |
| `-l`, `--limit <N>` | Maximum results (default: `10`) |
| `--include-description` | Also match descriptions |
| `--skill-dir <DIR>` | Search this directory instead of all discovered skills |
| `--format <FORMAT>` | `text` or `json` |

## Managing Skill Lifecycle

These commands share `--format text|json` and, where they read skills,
`--skill-dir <DIR>` to limit them to one directory.

### skill-diff

Compare one skill across Claude, Codex, Copilot, and Cursor:

```bash
skrills skill-diff commit
skrills skill-diff commit --format json -C 5
```

`-C`, `--context <N>` sets the context lines around each difference (default: `3`).

### skill-catalog

Browse every available skill:

```bash
skrills skill-catalog --search git --source codex --limit 20
```

Filters: `--search <TEXT>`, `--source claude|codex|copilot|cursor`,
`--category <TAG>`, and `--limit <N>` (default: `50`).

### skill-import

Import a skill from a URL, git repository, or local path:

```bash
skrills skill-import https://github.com/org/skills.git --target codex --dry-run
skrills skill-import ./my-skill --force
```

`--target` is `claude`, `codex`, `copilot`, or `cursor` (default: `claude`).
`--force` overwrites an existing skill; `--dry-run` writes nothing.

### sync-pull

Pull skill updates from a remote source:

```bash
skrills sync-pull --source https://github.com/org/skills.git --skill commit --dry-run
```

`--source` is a git repository, HTTP endpoint, or registry; `--skill`
limits the pull to one skill; `--target` takes the same values as
`skill-import` (default: `claude`).

### skill-deprecate

Mark a skill as deprecated:

```bash
skrills skill-deprecate old-skill --message "Merged into new-skill" --replacement new-skill
```

### skill-rollback

Revert a skill to an earlier version:

```bash
skrills skill-rollback commit                  # List available versions
skrills skill-rollback commit --version abc123 # Roll back to that version hash
```

### skill-score

Score skills on validation, completeness, and metrics:

```bash
skrills skill-score                            # Score every discovered skill
skrills skill-score commit
skrills skill-score --below-threshold 60       # Only skills scoring under 60
```

### skill-profile

Show execution statistics for one skill, or overall when no name is given:

```bash
skrills skill-profile commit --period 7
```

`--period <DAYS>` sets the window (default: `30`).

### skill-usage-report

Generate a usage report:

```bash
skrills skill-usage-report --period 7 --format json --output usage.json
```

`--format` is `text` or `json`; `--output` writes to a file instead of
stdout; `--period <DAYS>` defaults to `30`.

### pre-commit-validate

Validate skill files from a git pre-commit hook:

```bash
skrills pre-commit-validate --staged
skrills pre-commit-validate --target codex --skill-dir skills/
```

`--staged` checks only staged files. `--target` is `claude`, `codex`,
`copilot`, `all`, or `both` (default: `all`).

## Usage Analytics

### export-analytics

Write usage analytics to a file:

```bash
skrills export-analytics                         # ~/.skrills/analytics_cache.json
skrills export-analytics --output backup.json --force-rebuild
```

`--force-rebuild` rebuilds from session data even when a cache exists.

### import-analytics

Load analytics from an earlier export:

```bash
skrills import-analytics backup.json --overwrite
```

`--overwrite` replaces an existing cache file.

## Setup and Diagnostics

### doctor

Check your configuration:

```bash
skrills doctor
```

Verifies the Codex MCP registration in `~/.codex/config.toml`, where `skrills setup` writes it, and identifies problems. A legacy `~/.codex/mcp_servers.json` is inspected only when present.

### setup

Configure skrills for a specific client:

```bash
skrills setup --client codex              # Configure for Codex
skrills setup --client claude             # Configure for Claude
skrills setup --client both               # Configure for Claude and Codex
skrills setup --client all                # Configure every supported client
skrills setup --reinstall                 # Reconfigure from scratch
skrills setup --uninstall                 # Remove configuration
skrills setup --universal                 # Also sync to ~/.agents/skills
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--client` | `claude`, `codex`, `copilot`, `cursor`, `both` (Claude and Codex), or `all`. Prompts when omitted |
| `--bin-dir` | Override install location |
| `--reinstall` / `--uninstall` / `--add` | Control lifecycle |
| `--yes` | Skip confirmation prompts |
| `--universal` | Also sync skills to `~/.agents/skills` |
| `--mirror-source` | Override source directory for mirroring (default: `~/.claude`) |
| `--include-marketplace` | With `--universal`, also copy skills from `~/.claude/plugins/marketplaces` (uninstalled plugins). Off by default; `SKRILLS_INCLUDE_MARKETPLACE` sets it too |

### tui

Launch the interactive terminal interface:

```bash
skrills tui
skrills tui --skill-dir ~/.custom/skills
```

Provides a visual interface for sync management.

### dashboard

Real-time terminal dashboard for skill monitoring. Requires a TTY:

```bash
skrills dashboard --skill-dir ~/.custom/skills
```

### cold-window

Run the cold-window real-time analysis surface, as a TUI, a browser
dashboard, or both:

```bash
skrills cold-window --tui
skrills cold-window --browser --port 8888
skrills cold-window --browser --demo        # Built-in demo data
```

Options include `--alert-budget <N>` (default `100000`, minimum 2),
`--research-rate <N>`, `--tick-rate-ms <MILLIS>`, `--no-adaptive`,
`--no-bell`, `--skill-dir <DIR>`, and `--plugins-dir <DIR>`. See
[Cold-Window Real-Time Analysis](cold-window.md) for the full reference.

### cert

Manage TLS certificates for HTTPS transport:

```bash
skrills cert status                       # Show certificate validity and expiry
skrills cert renew                        # Regenerate self-signed certificate
skrills cert install my.pem --key my-key.pem        # Install custom certificate
```

**Subcommands:**

| Subcommand | Purpose |
|------------|---------|
| `status` | Display certificate path, validity, issuer, and days until expiry |
| `renew` | Generate a new self-signed certificate (365-day validity). Without `--force` it does nothing while the current certificate is valid for more than 30 days, and refuses to replace one that is not self-signed (a CA-issued certificate) |
| `install` | Import certificate and key from external files |

> **Note:** Certificate management is currently CLI-only. There is no MCP tool or plugin skill for cert operations yet. Use the `skrills cert` subcommands directly from a terminal.

## MCP Tools Reference

When running as an MCP server, skrills exposes these tools to your AI assistant:

### Core Tools

| Tool | Purpose |
|------|---------|
| `validate-skills` | Check skill compatibility |
| `analyze-skills` | Token usage and dependencies |
| `skill-metrics` | Aggregate statistics (quality, tokens, dependencies) |
| `sync-all` | Sync all configurations |
| `sync-status` | Preview changes (dry run) |

### Sync Tools

| Tool | Purpose |
|------|---------|
| `sync-from-claude` | Copy Claude skills to Codex |
| `sync-skills` | Sync skills between agents |
| `sync-commands` | Sync slash commands |
| `sync-mcp-servers` | Sync MCP configurations |
| `sync-preferences` | Sync preferences |

### Intelligence Tools

| Tool | Purpose |
|------|---------|
| `recommend-skills-smart` | Smart recommendations using dependencies, usage, and context |
| `analyze-project-context` | Analyze languages, frameworks, and keywords |
| `suggest-new-skills` | Identify skill gaps based on context and usage |
| `create-skill` | Create a new skill via GitHub search, LLM, or both |
| `search-skills-github` | Search GitHub for existing `SKILL.md` files |
| `resolve-dependencies` | Resolve direct/transitive dependencies or dependents |
| `recommend-skills` | Suggest related skills based on dependency relationships |

### Skill Tracing Tools

For debugging skill loading issues:

| Tool | Purpose |
|------|---------|
| `skill-loading-status` | Report skill roots, trace/probe install status, marker coverage |
| `enable-skill-trace` | Install trace/probe skills, optionally instrument with markers |
| `disable-skill-trace` | Remove trace/probe skill directories |
| `skill-loading-selftest` | Return probe line and expected response |

### Subagent Tools

When `subagents` feature is enabled:

| Tool | Purpose |
|------|---------|
| `list-subagents` | List available subagent specifications |
| `run-subagent` | Execute a subagent (cli or api mode) |
| `get-run-status` | Check status of a running subagent |

**Note:** `run-subagent` accepts `execution_mode` (`cli` or `api`) and an optional `cli_binary` override. When `execution_mode=cli`, `backend` is ignored.

### MCP Tool Input Examples

**sync-from-claude:**
```json
{}
```

**resolve-dependencies:**
```json
{ "uri": "skill://skrills/codex/my-skill/SKILL.md", "direction": "dependencies", "transitive": true }
```

### Smart Recommendation Workflows

**Project-aware recommendations:**
1. `analyze-project-context` → `recommend-skills-smart` → `suggest-new-skills`

**GitHub-assisted skill creation:**
1. `search-skills-github` → `create-skill` (use `dry_run: true` to preview)

## Skill Loading Validation

Use trace/probe tools when you need a deterministic signal that skills are loading in the current Claude Code or Codex session.

**Workflow:**

1. Call `enable-skill-trace` (use `dry_run: true` to preview). This installs two debug skills and can instrument skill files by appending `<!-- skrills-skill-id: ... -->` markers (with optional backups).

2. Restart the session if the client does not hot-reload skills.

3. Call `skill-loading-selftest` and send the returned `probe_line`. Expect `SKRILLS_PROBE_OK:<token>`.

4. With tracing enabled and markers present, each assistant response should end with `SKRILLS_SKILLS_LOADED: [...]` and `SKRILLS_SKILLS_USED: [...]`.

Use `skill-loading-status` to confirm which roots were scanned and whether markers are present. Use `disable-skill-trace` to remove debug skills when finished.

## Common Workflows

### Validate skills before committing

```bash
skrills validate --target both --errors-only
```

### Find and optimize large skills

```bash
skrills analyze --min-tokens 2000 --suggestions
```

### Set up a new machine

```bash
# Install
curl -LsSf https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.sh | sh

# Verify
skrills doctor

# Sync from your main setup
skrills sync-all --from claude
```

### Debug skill loading

These are MCP tools, not CLI subcommands. Ask the assistant to call them
from a Claude Code or Codex session connected to `skrills serve`:

1. Call `skill-loading-status` to check which roots were scanned.
2. Call `enable-skill-trace` to install the trace and probe skills.
3. Restart the session, then call `skill-loading-selftest`.

### CI-friendly validation

```bash
skrills validate --target codex --format json --errors-only
```
