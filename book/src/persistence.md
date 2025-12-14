# State Management and Persistence

`skrills` uses minimal persistent state, focusing on discovery caches and sync mirrors. This chapter describes what data is retained and how to inspect or reset it.

## What Is Persisted

### Skill Mirrors

Skill mirrors are stored in `~/.codex/skills-mirror/`. This directory contains copies of skills synced from `~/.claude/` (or `SKRILLS_MIRROR_SOURCE`) when running `skrills sync`, `skrills sync-all`, `skrills mirror`, or the `sync-from-claude` MCP tool. This keeps Codex and Claude skill sources aligned without modifying the originals.

Command files are mirrored byte-for-byte (non-UTF-8 safe) and can skip overwriting existing targets with `--skip-existing-commands`.

### Discovery Cache

The discovery cache is stored in `~/.codex/skills-cache.json` (configurable via `SKRILLS_CACHE_PATH`). It stores discovered skill metadata to prevent repeated directory traversals:

- TTL configurable via `SKRILLS_CACHE_TTL_MS` or `cache_ttl_ms` in the manifest
- Automatically refreshes when stale
- Live invalidation available with `--watch` flag on `skrills serve`

### Skills Manifest

The `~/.codex/skills-manifest.json` file controls skill discovery:

```json
{
  "priority": ["codex", "mirror", "claude", "agent"],
  "expose_agents": true,
  "cache_ttl_ms": 60000
}
```

### Subagent Configuration

If present, `~/.codex/subagents.toml` sets defaults for subagent execution:

```toml
default_backend = "codex"
```

The `SKRILLS_SUBAGENTS_DEFAULT_BACKEND` environment variable overrides `default_backend` at runtime.

## What Is NOT Persisted

- **User prompts**: Prompts are transient and never written to disk.
- **Validation results**: Run `skrills validate` each time; results are not cached.
- **Analysis results**: Run `skrills analyze` each time; results are not cached.

## Inspecting State

### View Discovered Skills

```bash
skrills validate --format json  # Shows all discovered skills with validation status
```

### Preview Sync Changes

```bash
skrills sync-status --from claude  # Shows what would be synced
```

### Diagnose Configuration

```bash
skrills doctor  # Verifies Codex MCP configuration
```

## Clean Resets

### Clear Skill Mirrors

Remove synced skills to read only from original directories:

```bash
rm -rf ~/.codex/skills-mirror/
```

### Clear Discovery Cache

Force re-discovery of all skills:

```bash
rm ~/.codex/skills-cache.json
```

### Full Reset

Remove all skrills state files:

```bash
rm -rf ~/.codex/skills-mirror/
rm ~/.codex/skills-cache.json
rm ~/.codex/skills-manifest.json
rm ~/.codex/subagents.toml
```
