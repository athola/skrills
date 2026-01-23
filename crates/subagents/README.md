# Subagents

This crate provides subagent capabilities for Skrills, enabling task delegation to specialized agents.

## Quick Start

1. **Build** with the `subagents` feature:
   ```bash
   cargo run -p skrills-server --features subagents -- serve
   ```

2. **Use** via MCP tools:
   - `list-subagents`: View available templates.
   - `run-subagent`: Execute a task (e.g., `{ "prompt": "list files", "backend": "codex" }`).
   - `get-run-status`: Check progress using `run_id`.

3. **Configure**: Set `SKRILLS_SUBAGENTS_DEFAULT_BACKEND=claude` to change the default adapter.

> **Note**: Async runs and secure transcripts require the Codex backend. WebSocket/HTTP streaming is planned.