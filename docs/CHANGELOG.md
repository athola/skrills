# Changelog

## 0.1.1 - 2025-11-24
- Added single-command installer that also registers the Codex hook/MCP server by default (opt-out/env toggles).
- Introduced mdBook documentation and GitHub Pages deploy workflow.
- Added CI workflow (fmt, clippy, tests, docs, mdbook) and refined release asset naming.
- Expanded Makefile with docs/book targets and CLI demos.
- Documented child-process safety, cache TTL manifest option, and installer defaults.

## 0.1.0 - 2025-11-24
- Split into a Rust workspace: `crates/core` (library/MCP server) and `crates/cli` (binary wrapper).
- Added structured `_meta` outputs across tools with priority ranks and duplicate info.
- Synced AGENTS.md generation to include per-skill `priority_rank` and overall priority list.
- Enhanced README with clearer install/usage, universal sync, TUI, and structured output examples.

## 0.1.0 (earlier changes)
- Exposed AGENTS.md as `doc://agents` with manifest/env opt-out.
- Added universal sync helper and hook installer flags.
- Improved duplicate handling and diagnostics during autoload.
