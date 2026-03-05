# skrills-server

Core library powering the `skrills` MCP server and CLI. It handles discovery, filtering, manifest construction, pinning, and diagnostics for local `SKILL.md` files.

The HTTP transport serves the MCP protocol alongside a browser dashboard (Leptos SSR) and REST API endpoints for skills and metrics. Routes are merged into a single Axum router with shared middleware for auth, CORS, and request IDs.

This crate is intended to be used by the `skrills` binary, but it can also be embedded in custom MCP-compatible tooling.

## Project resources
- Source & issues: https://github.com/athola/skrills
- User docs: https://github.com/athola/skrills/tree/master/book
