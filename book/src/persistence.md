# Understanding Skrills Data

Skrills stores minimal data on disk. This chapter explains what gets saved, where to find it, and how to reset things when needed.

## What Skrills Saves

### Your Skills

Skrills discovers skills from these directories:

| Location | Purpose |
|----------|---------|
| `~/.codex/skills/` | Codex CLI skills |
| `~/.claude/skills/` | Claude Code skills |
| `~/.agent/skills/` | Universal agent skills |

When you run `skrills sync` or `skrills mirror`, skrills copies skills between these directories so both tools can access them.

**Note:** Codex skills require an experimental feature flag in `~/.codex/config.toml`:

```toml
[features]
skills = true
```

### Discovery Cache

To avoid scanning directories repeatedly, skrills caches skill metadata:

**Location:** `~/.codex/skills-cache.json`

The cache refreshes automatically when:
- The time-to-live (TTL) expires (default: 60 seconds)
- You use the `--watch` flag with `skrills serve`

### Manifest

The manifest controls which directories skrills searches and in what order:

**Location:** `~/.codex/skills-manifest.json`

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
| `expose_agents` | Include agent definitions in discovery |
| `cache_ttl_ms` | How long to cache results (milliseconds) |

### Subagent Configuration

Settings for launching subagents:

**Location:** `~/.claude/subagents.toml` or `~/.codex/subagents.toml`

```toml
execution_mode = "cli"
cli_binary = "auto"
default_backend = "codex"
```

| Field | Purpose |
|-------|---------|
| `execution_mode` | `cli` (run commands) or `api` (use API) |
| `cli_binary` | Which CLI to use: `claude`, `codex`, or `auto` |
| `default_backend` | Default API backend when using API mode |

## What Skrills Does NOT Save

Skrills keeps these things temporary:

- **Validation results** — Run `skrills validate` each time
- **Analysis results** — Run `skrills analyze` each time
- **User prompts** — Never written to disk

## Checking What's Stored

### See discovered skills

```bash
skrills validate --format json
```

### Preview sync changes

```bash
skrills sync-status --from claude
```

### Verify configuration

```bash
skrills doctor
```

## Resetting Things

### Clear synced skills

Remove skills that were copied from another tool:

```bash
rm -rf ~/.codex/skills/
```

### Clear the cache

Force skrills to re-scan all directories:

```bash
rm ~/.codex/skills-cache.json
```

### Full reset

Remove all skrills state and start fresh:

```bash
rm -rf ~/.codex/skills/
rm -rf ~/.codex/skills-mirror/
rm ~/.codex/skills-cache.json
rm ~/.codex/skills-manifest.json
rm ~/.codex/subagents.toml
rm ~/.claude/subagents.toml
```

After a full reset, run `skrills setup` to reconfigure.

## Legacy Files

Older versions of skrills used `~/.codex/skills-mirror/` to store a complete copy of Claude assets. Current versions sync directly to `~/.codex/skills/` instead. You can safely delete the mirror directory:

```bash
rm -rf ~/.codex/skills-mirror/
```
