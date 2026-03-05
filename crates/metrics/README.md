# skrills-metrics

SQLite-based metrics collection for skrills skill invocations, validations, and sync events.

## Features

- Lightweight SQLite storage for metrics
- Tracks skill invocations, validation results, and sync operations
- Async-friendly with Tokio support

## Usage

```rust
use skrills_metrics::MetricsCollector;

let collector = MetricsCollector::new()?;
collector.record_skill_invocation("my-skill", 150, true, Some(1024))?;
```

## License

MIT
