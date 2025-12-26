# CLI Reference

This reference organizes commands by what you want to accomplish.

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
| `--target <TARGET>` | `claude`, `codex`, or `both` (default: `both`) |
| `--autofix` | Add missing frontmatter automatically |
| `--backup` | Create backups before autofix |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |
| `--errors-only` | Hide passing skills |

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
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--from` | Source side: `claude` or `codex` (default: `claude`) |
| `--dry-run` | Preview changes without writing |
| `--skip-existing-commands` | Keep existing commands on target side |

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

Copy Claude assets to Codex defaults and refresh the agent list:

```bash
skrills mirror                           # Full mirror
skrills mirror --skip-existing-commands  # Preserve existing commands
skrills mirror --dry-run                 # Preview changes (hashes sources, reports intended writes)
```

## Running the MCP Server

### serve

Start skrills as an MCP server (used by Claude Code and Codex CLI):

```bash
skrills serve                             # Standard startup
skrills serve --watch                     # Auto-reload on file changes (requires watch feature)
skrills serve --cache-ttl-ms 300000       # 5-minute cache
skrills serve --skill-dir ~/.custom/skills  # Custom skill directory
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--skill-dir <DIR>` | Additional skill directory to include |
| `--cache-ttl-ms <N>` | Discovery cache TTL in milliseconds |
| `--watch` | Enable live filesystem invalidation |

The MCP server exposes validation, analysis, sync, and recommendation tools directly to your AI assistant.

## Working with Subagents

### agent

Launch a saved agent by name:

```bash
skrills agent codex-dev                   # Run the codex-dev agent
skrills agent my-agent --dry-run          # Preview without running
skrills agent my-agent --skill-dir ~/.custom/skills  # Include custom skills
```

**Execution mode configuration:**

Subagent behavior comes from multiple sources (in priority order):
1. `SKRILLS_SUBAGENTS_EXECUTION_MODE` environment variable
2. `execution_mode` in `~/.claude/subagents.toml` or `~/.codex/subagents.toml`
3. Default: `cli`

When `execution_mode=api` and no backend is specified, skrills checks:
1. `default_backend` in the config file
2. `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` environment variable
3. Default: `codex`

In `cli` mode, skrills uses `cli_binary` (or `SKRILLS_CLI_BINARY`), auto-detecting the current client via CLI env or server path (fallback: `claude`).

### sync-agents

Update the available skills list for agents:

```bash
skrills sync-agents                       # Refresh AGENTS.md
skrills sync-agents --path custom.md      # Write to different file
```

**Skill naming caveat:**

Skills are named from the `name:` field in `SKILL.md` frontmatter. Treat these names as opaque strings—they may include punctuation like `:` for namespacing (e.g., `pensive:shared`).

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

Generate a new skill via GitHub search, LLM generation, or both:

```bash
skrills create-skill my-new-skill --description "Helps with testing"
skrills create-skill my-new-skill --description "..." --method github
skrills create-skill my-new-skill --description "..." --dry-run  # Preview
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--description <TEXT>` | Required description of the skill |
| `--method <METHOD>` | `github`, `llm`, or `both` (default: `both`) |
| `--target-dir <DIR>` | Where to save (default: installed client, Claude preferred) |
| `--project-dir <DIR>` | Project directory for context |
| `--dry-run` | Preview without writing files |
| `--format <FORMAT>` | `text` or `json` (default: `text`) |

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

## Setup and Diagnostics

### doctor

Check your configuration:

```bash
skrills doctor
```

Verifies Codex MCP configuration and identifies problems.

### setup

Configure skrills for a specific client:

```bash
skrills setup --client codex              # Configure for Codex
skrills setup --client claude             # Configure for Claude
skrills setup --client both               # Configure for both
skrills setup --reinstall                 # Reconfigure from scratch
skrills setup --uninstall                 # Remove configuration
skrills setup --universal                 # Also sync to ~/.agent/skills
```

**Options:**

| Option | Purpose |
|--------|---------|
| `--client` | `claude`, `codex`, or `both` |
| `--bin-dir` | Override install location |
| `--reinstall` / `--uninstall` / `--add` | Control lifecycle |
| `--yes` | Skip confirmation prompts |
| `--universal` | Also mirror skills to `~/.agent/skills` |
| `--mirror-source` | Override source directory for mirroring (default: `~/.claude`) |

### tui

Launch the interactive terminal interface:

```bash
skrills tui
skrills tui --skill-dir ~/.custom/skills
```

Provides a visual interface for sync management.

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

```bash
# Check status
skrills skill-loading-status

# Enable tracing
skrills enable-skill-trace

# Restart session, then validate
skrills skill-loading-selftest
```

### CI-friendly validation

```bash
skrills validate --target codex --format json --errors-only
```
