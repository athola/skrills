# skrills-metrics

SQLite-based metrics collection for skrills skill invocations, validations, and sync events.

## Features

- Lightweight SQLite storage for metrics
- Tracks skill invocations, validation results, and sync operations
- Async-friendly with Tokio support

## Usage

```rust
use skrills_metrics::MetricsStore;

let store = MetricsStore::open_default().await?;
store.record_invocation("my-skill", duration, success).await?;
```

## License

MIT
