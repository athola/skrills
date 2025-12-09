# Subagents MCP quick example

1. Build with subagents feature:
```bash
cargo run -p skrills-server --features subagents -- serve
```

2. From a MCP client, call:
- `list_subagents` to see available templates.
- `run_subagent` with `{ "prompt": "list files" , "backend": "codex" }`.
- `get_run_status` with the returned `run_id`.

3. Optional: set `SKRILLS_SUBAGENTS_DEFAULT_BACKEND=claude` to default to Claude-style adapter.

Notes: Async runs and secure transcripts are Codex-only; WebSocket/HTTP streaming is planned (issue #25).
