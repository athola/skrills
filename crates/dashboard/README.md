# skrills-dashboard

Terminal UI and browser dashboard for skrills skill visualization and metrics.

## Features

- Real-time skill invocation metrics
- Validation status overview
- Sync operation tracking
- TUI built with Ratatui for terminal rendering
- Browser UI rendered via Leptos SSR, served alongside MCP on the HTTP transport

## Public Modules

The `app` and `ui` modules are re-exported as `pub` for embedding in custom tooling:

- `app` — `App` state machine and `Dashboard` entry point
- `ui` — Ratatui rendering functions

## Usage

Access via the `skrills` CLI:

```bash
# Terminal dashboard
skrills tui

# Browser dashboard (served on the MCP HTTP port)
skrills serve
# then open http://localhost:<port>/ in a browser
```

Or embed as a library:

```rust
use skrills_dashboard::Dashboard;

let dashboard = Dashboard::new()?;
dashboard.run().await?;
```

## License

MIT
