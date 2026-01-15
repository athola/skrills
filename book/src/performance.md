# Performance Baseline and Tuning

The `skrills` MCP server has minimal overhead. Validation, analysis, and sync operations are designed to be efficient.

## Expected Performance

The initial skill directory scan is cached to avoid repeated filesystem access. Validation processes skills in parallel where possible, and analysis uses approximate but fast token counting. Sync operations use content hashing to skip unchanged files and perform efficient byte-for-byte copies.

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

Investigate performance if validation feels slow, which often indicates skill directories containing many files. If sync is slow, use `sync-status` to identify large change sets. Slow startup times might require increasing the cache TTL or reducing the number of monitored skill directories.
