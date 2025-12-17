# Persistence and State

This document details `skrills`'s persistence mechanisms: what data is persisted, where it is stored, and how users can inspect or reset it.

## What Gets Persisted

### Codex Skills (Discovery Root)

Codex discovers skills from `~/.codex/skills/**/SKILL.md` (recursive). When you run `skrills sync` or `skrills mirror`, skrills copies `SKILL.md` skills (and their adjacent supporting files) into `~/.codex/skills/` so Codex can load them.

Codex skills are behind an experimental feature flag in `~/.codex/config.toml`:

```toml
[features]
skills = true
```

### Skill Mirrors (Optional)

`~/.codex/skills-mirror/` is a legacy/optional directory used by older versions of `skrills` to keep a full, byte-for-byte mirror of Claude assets. Current `skrills` releases sync skills into `~/.codex/skills/` and agents into `~/.codex/agents/` without creating `~/.codex/skills-mirror/` by default.

### Discovery Cache

The discovery cache is stored in `~/.codex/skills-cache.json` (configurable via `SKRILLS_CACHE_PATH`). It stores discovered skill metadata to prevent repeated directory traversals. The cache:
- Has a configurable TTL via `SKRILLS_CACHE_TTL_MS` or `cache_ttl_ms` in the manifest
- Automatically refreshes when stale or when the `--watch` flag is used
- Can be bypassed with a different `SKRILLS_CACHE_PATH` for testing

### Skills Manifest

The `~/.codex/skills-manifest.json` file stores discovery configuration:

```json
{
  "priority": ["codex", "mirror", "claude", "agent"],
  "expose_agents": true,
  "cache_ttl_ms": 60000
}
```

### Subagents Configuration

The `~/.codex/subagents.toml` file stores subagent defaults:

```toml
default_backend = "codex"
```

## What Is NOT Persisted

- **User prompts**: Prompts are transient and never written to disk.
- **Validation results**: Run `skrills validate` each time; results are not cached.
- **Analysis results**: Run `skrills analyze` each time; results are not cached.

## Inspecting State

### Check Discovered Skills

```bash
skrills validate --format json  # Shows all discovered skills with validation status
```

### Preview Sync Status

```bash
skrills sync-status --from claude  # Shows what would be synced
```

### Diagnose Configuration

```bash
skrills doctor  # Verifies Codex MCP configuration
```

## Resetting State

### Clear Codex Skills

```bash
rm -rf ~/.codex/skills/
```

### Clear Skill Mirrors

```bash
rm -rf ~/.codex/skills-mirror/
```

### Clear Discovery Cache

```bash
rm ~/.codex/skills-cache.json
```

### Reset to Defaults

Remove all skrills state files:

```bash
rm -rf ~/.codex/skills/
rm -rf ~/.codex/skills-mirror/
rm ~/.codex/skills-cache.json
rm ~/.codex/skills-manifest.json
rm ~/.codex/subagents.toml
```

## Operational Tips

1. **Isolate test environments**: Set `SKRILLS_CACHE_PATH` to a temp file for parallel testing.
2. **Sync regularly**: Run `skrills sync-all` to keep configurations in sync.
3. **Validate after sync**: Run `skrills validate` after syncing to catch compatibility issues.
4. **Use dry-run**: Preview changes with `--dry-run` before syncing.
