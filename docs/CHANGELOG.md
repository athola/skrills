# Changelog

## Unreleased
- Documented MCP runtime tools (`runtime-status`, `set-runtime-options`) with config precedence, CLI `--version`/`--help` examples, and added SemVer/stability notes.
- Added `render-preview` MCP tool to preview matched skills, manifest size, and token estimates before injecting `additionalContext`.
- Added public API guardrail job (cargo-public-api) and contributing note about semver checks.
- Split monolithic core into crates: `discovery` (scan/hash), `state` (persistence/env), `server` (CLI+MCP), `cli` (thin bin).
- Added coverage workflow (`cargo llvm-cov` + Codecov) and README badges; documented local coverage command.
- Added smoke tests for discovery hashing and state persistence/auto-pin.
- Installers now clean legacy `codex-mcp-skills` binaries and MCP config entries before wiring skrills to avoid conflicts.

## 0.1.14 - 2025-11-25
- Added `skrills doctor` to inspect Codex MCP config (`mcp_servers.json` and `config.toml`) and verify `type = "stdio"` plus binary paths.
- Hardened MCP tool schemas to always include `type = "object"` so Codex no longer raises `missing field "type"` during startup.
- `serve` gains `--trace-wire` / `SKRILLS_TRACE_WIRE=1` to hex+UTF-8 log MCP initialize traffic; warm-up now defers until after handshake and scans log when slow.
- Installers now write `type = "stdio"` to both Codex config files, tolerate permission failures, add POSIX `--local` build flag, and keep PowerShell parity.

## 0.1.13 - 2025-11-25
- Installer filters release archives more strictly and uses a source build as a secondary option when no match is found.
- CI jobs are now gated by relevant path changes to avoid unnecessary runs.

## 0.1.12 - 2025-11-25
- Hardened installer release-asset lookup for resiliency across GitHub responses.

## 0.1.11 - 2025-11-25
- Release workflow now skips asset uploads when a release already exists, preventing duplicate publishes.

## 0.1.10 - 2025-11-25
- Release workflow creates the GitHub release before uploading assets to eliminate race conditions.

## 0.1.9 - 2025-11-25
- Fixed release upload include patterns to ensure platform archives are attached correctly.

## 0.1.8 - 2025-11-25
- Respected manifest flag when hiding agents documentation in autoload outputs.
- Cached cargo artifacts in the audit workflow to speed up security checks.
- Corrected release include paths across workflows.

## 0.1.7 - 2025-11-25
- Added release dry-run builds and cache reuse in CI to validate artifacts ahead of tagging.

## 0.1.6 - 2025-11-24
- Switched to supported archive options in the release workflow for better cross-platform packaging.

## 0.1.5 - 2025-11-24
- Set the ZIP flag in the Windows upload step to produce valid Windows artifacts.

## 0.1.4 - 2025-11-24
- Fixed CI upload action inputs for Rust binaries.
- Stabilized core tests by fixing race conditions around environment variable handling.

## 0.1.3 - 2025-11-24
- Installer builds from source in an isolated cargo home as a secondary option when release assets are missing, then installs into the requested bin dir.
- Asset selection favors `jq` with an awk secondary (no Python) and optional `pipefail` for POSIX shells.
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
