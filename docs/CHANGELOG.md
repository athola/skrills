# Changelog

## 0.1.3 - 2025-11-24
- Installer falls back to building from source in an isolated cargo home when release assets are missing, then installs into the requested bin dir.
- Asset selection favors `jq` with an awk fallback (no Python) and optional `pipefail` for POSIX shells.
- Release workflow asset names now use the action template placeholder (`{{ target }}`) to publish platform tarballs reliably.

## 0.1.2 - 2025-11-24
- Added social/brand icon (8-bit 1280×640) at `assets/icon.png` and linked it in README.
- Installer docs now use branch-agnostic `/HEAD/` URLs; README/book/FAQ updated.
- Makefile `book` target builds and opens the mdBook (replaces book-open).
- Added comparison + FAQ chapters to the book and FAQ to `docs/`.
- Release workflow assets now interpolate target correctly; audit workflow runs `cargo audit` directly.

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
