# Performance Baseline and Tuning

The `skrills` MCP server has minimal overhead. Validation, analysis, and sync operations are designed to be efficient.

## Expected Performance

- **Skill discovery**: Initial skill directory scan is cached to avoid repeated filesystem access
- **Validation**: Processes skills in parallel where possible
- **Analysis**: Token counting is approximate but fast
- **Sync**: Byte-for-byte file copy is efficient; uses content hashing to skip unchanged files

These figures are from measurements on an M1 Pro system with a typical skill set.

## Tuning Recommendations

### Cache TTL

Configure `SKRILLS_CACHE_TTL_MS` or `cache_ttl_ms` in the manifest to balance freshness with performance:

```bash
# Longer TTL for stable skill sets
export SKRILLS_CACHE_TTL_MS=300000  # 5 minutes
```

### Validation Performance

Use filtering options to reduce work:

```bash
# Only check for errors
skrills validate --errors-only

# Target specific directory
skrills validate --skill-dir ~/my-skills
```

### Analysis Performance

Filter to focus on relevant skills:

```bash
# Only analyze large skills
skrills analyze --min-tokens 1000
```

## When to Investigate

- If validation is slow, check if skill directories contain many files
- If sync is slow, use `sync-status` to identify large change sets
- If startup is slow, increase cache TTL or reduce number of skill directories
