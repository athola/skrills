# Observability and Audit Logging

`skrills` provides structured logging via the `tracing` crate for debugging and visibility.

## Logging Overview

- **Autoload Events**: Logs skill matching, content loading, and injection to show which skills are selected for each prompt.
- **Cache Operations**: Cache hits, misses, and invalidation help diagnose performance and freshness issues.
- **MCP Protocol Events**: Tool invocations and responses are logged to debug integration issues.

## Log Configuration

Set the `RUST_LOG` environment variable to control log verbosity:

```bash
# Enable debug logging for skrills
RUST_LOG=skrills=debug skrills serve

# Enable trace logging for detailed diagnostics
RUST_LOG=skrills=trace skrills serve
```

## Audit Logging

Key events to monitor include:
- Skill discovery and loading outcomes.
- Pin/unpin operations.
- Cache invalidation events.
- MCP tool invocations and their results.

### Best Practices

- Use structured logging with consistent field names for easier log aggregation.
- Monitor `autoload_bytes` to identify potential skill bloat.
- Enable `render_mode_log` via `set-runtime-options` when debugging truncation issues.

## Log Triage

- Index logs by `skill_name`, `prompt_hash`, and `error` for efficient searching.
- Correlate Claude Code session events with skrills logs using timestamps.
