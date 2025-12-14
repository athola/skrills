# Runtime Configuration

This chapter covers the configuration options available for tuning skrills behavior.

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SKRILLS_MIRROR_SOURCE` | Mirror source root directory | `~/.claude` |
| `SKRILLS_CACHE_TTL_MS` | Discovery cache TTL in milliseconds | `60000` |
| `SKRILLS_CACHE_PATH` | Override skills cache file path | `~/.codex/skills-cache.json` |
| `SKRILLS_CLIENT` | Force installer target (`codex` or `claude`) | Auto-detected |
| `SKRILLS_NO_MIRROR` | Skip post-install mirror on Codex (`1` to enable) | Disabled |
| `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` | Default subagent backend (`codex` or `claude`) | `codex` |

## Configuration Files

### Skills Manifest

The `~/.codex/skills-manifest.json` file controls skill discovery:

```json
{
  "priority": ["codex", "mirror", "claude", "agent"],
  "expose_agents": true,
  "cache_ttl_ms": 60000
}
```

Fields:
- `priority`: Order of skill directory precedence
- `expose_agents`: Whether to expose agent definitions
- `cache_ttl_ms`: Cache time-to-live in milliseconds

### Subagents Configuration

The `~/.codex/subagents.toml` file configures subagent defaults:

```toml
default_backend = "codex"
```

## CLI Hints

- **Check version**: `skrills --version`
- **List commands**: `skrills --help`
- **Diagnose setup**: `skrills doctor`

## Practical Recipes

### Speed Up Discovery

Increase cache TTL for stable skill sets:

```bash
export SKRILLS_CACHE_TTL_MS=300000  # 5 minutes
```

### Use Custom Mirror Source

Point to a different Claude installation:

```bash
export SKRILLS_MIRROR_SOURCE=/path/to/custom/claude
skrills sync-all
```

### Isolate Test Environments

Use separate cache files for parallel testing:

```bash
export SKRILLS_CACHE_PATH=/tmp/test-skills-cache.json
skrills validate --target codex
```
