# Configuration

Customize skrills behavior through environment variables and configuration files.

## Quick Reference

| Variable | What it does | Default |
|----------|--------------|---------|
| `SKRILLS_CLIENT` | Force target: `codex` or `claude` | Auto-detected |
| `SKRILLS_CACHE_TTL_MS` | How long to cache skill discovery | `60000` (1 min) |
| `SKRILLS_MIRROR_SOURCE` | Where to copy skills from | `~/.claude` |

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

**`SKRILLS_NO_MIRROR`**

Skip automatic skill syncing during installation.

- Default: disabled
- Set to `1` to skip

```bash
# Install without syncing Claude skills
SKRILLS_NO_MIRROR=1 ./scripts/install.sh
```

### Client Settings

**`SKRILLS_CLIENT`**

Force skrills to target a specific client instead of auto-detecting.

- Default: auto-detected from `~/.claude` or `~/.codex` presence
- Values: `claude` or `codex`

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
