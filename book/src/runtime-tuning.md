# Configuration

Customize skrills behavior through environment variables and configuration files.

## Quick Reference

| Variable | What it does | Default |
|----------|--------------|---------|
| `SKRILLS_CLIENT` | Force target: `codex` or `claude` | Auto-detected |
| `SKRILLS_CACHE_TTL_MS` | How long to cache skill discovery | `60000` (1 min) |
| `SKRILLS_MIRROR_SOURCE` | Where to copy skills from | `~/.claude` |
| `SKRILLS_INCLUDE_CLAUDE` | Discover skills in `~/.claude/skills` and the Claude plugin cache (MCP server, dashboard, `/api/skills` and CLI alike) | Off |
| `SKRILLS_OFFLINE` | `1`/`true`/`yes`/`on` never uses the network; `0`/`false`/`no`/`off` skips the connectivity probe | Probed on first network use |

## Environment Variables

### Discovery Settings

**`SKRILLS_CACHE_TTL_MS`**

How long skrills caches discovered skills before re-scanning directories.

- Default: `60000` (60 seconds)
- Use higher values if your skills rarely change
- Use lower values during development

```bash
# Cache for 5 minutes (stable skill set)
export SKRILLS_CACHE_TTL_MS=300000

# Cache for 10 seconds (active development)
export SKRILLS_CACHE_TTL_MS=10000
```

**`SKRILLS_CACHE_PATH`**

Where to store the discovery cache file.

- Default: `~/.codex/skills-cache.json`
- Useful for isolating test environments

```bash
# Use separate cache for testing
export SKRILLS_CACHE_PATH=/tmp/test-skills-cache.json
```

### Sync Settings

**`SKRILLS_MIRROR_SOURCE`**

Which directory to copy skills from during sync operations.

- Default: `~/.claude`
- Change this to sync from a different Claude installation

```bash
# Sync from a custom Claude setup
export SKRILLS_MIRROR_SOURCE=/path/to/other/claude
skrills sync-all
```

The installer syncs no skills unless `SKRILLS_UNIVERSAL=1` is set; see [Installation](installation.md#customizing-installation) for its variables.

### Offline Mode

**`SKRILLS_OFFLINE`**

`validate`, `analyze` and `sync` never touch the network, so they work offline with no setting. Features that do need it (GitHub skill search in `search-skills-github` and `create-skill`, and the `search-papers`, `search-discussions`, `resolve-doi` and `fetch-pdf` MCP tools) check connectivity once per process, the first time they run. Offline, they make no request and serve the newest stored answer:

```text
offline — using cached data (cached 2 hours ago): Found 3 papers
```

With nothing stored for that query they fail with `offline mode: no cached data for ...`. `fetch-pdf` serves a PDF already on disk and otherwise refuses to download.

- Truthy values (`1`, `true`, `yes`, `on`) force offline mode without probing. `skrills --offline <command>` does the same.
- Falsy values (`0`, `false`, `no`, `off`) assert the network is reachable and skip the probe, for hosts that block the probe targets (`1.1.1.1:443`, `8.8.8.8:443`).
- Answers are stored in the research cache (`research.db` under the user cache directory, in `skrills-tome`) each time an online call succeeds. Entries keep their store time, and offline mode serves them past their expiry, so the age in the notice is the one to judge by.

### Client Settings

**`SKRILLS_CLIENT`**

Force skrills to target a specific client instead of auto-detecting.

- Default: auto-detected from `~/.claude` or `~/.codex` presence
- Values: `claude` or `codex`
- The installer also reads it to pick the client for `skrills setup`, where `copilot`, `cursor`, `both` and `all` are accepted too

```bash
# Always target Claude
export SKRILLS_CLIENT=claude
```

### Subagent Settings

**`SKRILLS_SUBAGENTS_EXECUTION_MODE`**

How subagents run: as CLI commands or through the API.

- Default: `cli`
- Values: `cli` or `api`

**`SKRILLS_SUBAGENTS_DEFAULT_BACKEND`**

Which backend to use for API-mode subagents.

- Default: `codex`
- Values: `codex` or `claude`

**`SKRILLS_CLI_BINARY`**

Which CLI binary to use for subagent execution.

- Default: `auto` (detects current client)
- Values: `claude`, `codex`, or `auto`

## Configuration Files

### Skills Manifest

**Location:** `~/.codex/skills-manifest.json`

Controls skill discovery priority and caching:

```json
{
  "priority": ["codex", "mirror", "claude", "agent"],
  "expose_agents": true,
  "cache_ttl_ms": 60000
}
```

| Field | Purpose |
|-------|---------|
| `priority` | Search order for skill directories |
| `expose_agents` | Include agent definitions |
| `cache_ttl_ms` | Cache duration (overridden by `SKRILLS_CACHE_TTL_MS`) |

### Subagents Configuration

**Location:** `~/.claude/subagents.toml` or `~/.codex/subagents.toml`

```toml
execution_mode = "cli"
cli_binary = "auto"
default_backend = "codex"
```

The `auto` setting for `cli_binary` picks the right CLI based on:
1. `SKRILLS_CLIENT` environment variable
2. CLI environment indicators (CLAUDE/CODEX)
3. Server binary path
4. Fallback to `claude`

## Practical Recipes

### Speed up discovery for stable skills

If your skills rarely change, increase the cache time:

```bash
export SKRILLS_CACHE_TTL_MS=300000  # 5 minutes
```

### Isolate test environments

Use separate caches when running tests in parallel:

```bash
export SKRILLS_CACHE_PATH=/tmp/test-$$.json
skrills validate --target codex
```

### Sync from a different Claude installation

Point to a custom source directory:

```bash
export SKRILLS_MIRROR_SOURCE=/path/to/team/claude
skrills sync-all
```

### Force a specific target client

Override auto-detection:

```bash
export SKRILLS_CLIENT=claude
skrills setup
```

## Common Commands

```bash
skrills --version     # Check version
skrills --help        # List commands
skrills doctor        # Diagnose configuration
```
