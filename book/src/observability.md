# Observability and Audit Logging

`skrills` provides structured logging via the `tracing` crate for debugging and visibility.

## Logging Overview

- **Validation Events**: Logs skill validation, autofix operations, and compatibility checks.
- **Analysis Events**: Logs token counting, dependency analysis, and optimization suggestions.
- **Sync Operations**: Logs skill mirroring, command sync, and configuration changes.
- **Cache Operations**: Cache hits, misses, and invalidation help diagnose performance and freshness issues.
- **MCP Protocol Events**: Tool invocations and responses are logged to debug integration issues.

## Log Configuration

Set the `RUST_LOG` environment variable to control log verbosity:

```bash
# Enable debug logging for skrills
RUST_LOG=skrills=debug skrills serve

# Enable trace logging for detailed diagnostics
RUST_LOG=skrills=trace skrills serve

# Enable wire-level MCP tracing
skrills serve --trace-wire
```

## Audit Logging

Key events to monitor include:
- Skill discovery and validation outcomes
- Sync operations and their results
- Cache invalidation events
- MCP tool invocations and their results

### Best Practices

- Use structured logging with consistent field names for easier log aggregation
- Monitor validation errors to identify skill compatibility issues
- Track sync operations to detect configuration drift

## Log Triage

- Index logs by `skill_name`, `validation_target`, and `error` for efficient searching
- Correlate Claude Code session events with skrills logs using timestamps
