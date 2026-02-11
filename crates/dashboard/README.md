# skrills-dashboard

Terminal UI dashboard for skrills metrics and monitoring.

## Features

- Real-time skill invocation metrics
- Validation status overview
- Sync operation tracking
- Built with Ratatui for terminal rendering

## Usage

The dashboard is typically accessed through the main `skrills` CLI or can be used as a library:

```rust
use skrills_dashboard::Dashboard;

let dashboard = Dashboard::new()?;
dashboard.run().await?;
```

## License

MIT
