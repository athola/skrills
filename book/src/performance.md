# Performance Baseline and Tuning

The `skrills` MCP server has minimal overhead when integrated with Claude Code via hooks. The autoload process minimizes latency in skill selection and injection.

## Expected Autoload Overhead

- **Typical latency**: 5–15 ms for skill filtering and manifest generation
- **Skill discovery**: Initial skill directory scan is cached to avoid repeated filesystem access

These figures are from measurements on an M1 Pro system with a small skill set.

## How to Measure

1. Enable diagnostic logging with the `--diagnose` flag on `emit-autoload`.
2. Use `render-preview` to inspect matched skills and byte estimates without full content rendering.
3. Monitor the `autoload_bytes` output to track skill content sizes.

## Tuning Recommendations

- **`--max-bytes`**: Primary parameter for controlling payload size. Smaller values reduce context window usage but may truncate skill content.
- **`embed_threshold`**: Controls the similarity threshold for skill matching (default: 0.3). Higher values result in fewer but more relevant matches.
- **Cache TTL**: Configure `SKRILLS_CACHE_TTL_MS` or `cache_ttl_ms` in the manifest to balance freshness with performance.

## When to Investigate

- If `autoload_bytes` consistently approaches `max_bytes`, pin critical skills to ensure they are always included.
- If skill matching seems to miss relevant content, lower the `embed_threshold` value.
- If startup is slow, check if the skill directories contain an excessive number of files.
