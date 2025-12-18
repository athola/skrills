# Changelog

## 0.3.3 - 2025-12-18
- **NEW: Metrics Command**: Added `skrills metrics` CLI command and `skill-metrics` MCP tool for aggregate statistics including quality distribution, dependency graphs, and token usage.
- **NEW: Dependency Graph**: Added `DependencyGraph` module for skill relationship tracking with transitive resolution and cycle detection.
- **NEW: Resolve Dependencies Tool**: Added `resolve-dependencies` MCP tool for querying skill dependencies and dependents.
- **NEW: Dependency Validation**: Added `check_dependencies` flag to `validate-skills` for verifying skill dependencies exist.
- **NEW: Makefile Targets**: Added `make status`, `make install`, `make test-coverage`, `make security`, and `make deps-update` for developer workflows.

## 0.3.2 - 2025-12-17
- **NEW: Dependency Resolution System**: Comprehensive skill dependency tracking and resolution via YAML frontmatter.
- **Dependency Syntax**: Support for simple, structured, and compact syntax forms in frontmatter.
- **Resolution Engine**: Circular dependency detection, semver version constraints, and source pinning (e.g., `codex:base-skill`).
- **Optional Dependencies**: Configurable behavior for optional dependencies.
- **Extended Validation**: Dependency-related validation issues added to the validate crate.
- **NEW: Skill Loading Trace**: Diagnostic tools (`skill-loading-status`, `enable-skill-trace`, `disable-skill-trace`, `skill-loading-selftest`) for debugging skill loading.
- **Discovery & Sync Updates**: Updated discovery and sync modules for dependency support.
- **Documentation**: Added ADR 0002 for dependency resolution architecture and comprehensive docs.
- **BREAKING**: rmcp updated to 0.10; removed deprecated `info()` method from ServerHandler impl.
- **BREAKING**: `SkillSource` enum requires wildcard pattern matching due to `#[non_exhaustive]`.

## 0.3.1 - 2025-12-13
- **NEW: Validation Crate** (`skrills-validate`): Validate skills for Claude Code (permissive) and Codex CLI (strict frontmatter requirements). Includes auto-fix capability to add missing frontmatter.
- **NEW: Analysis Crate** (`skrills-analyze`): Token counting, dependency analysis, and optimization suggestions for skills.
- **NEW: Validation Integration**: Sync operations now optionally validate skills with `--validate` and `--autofix` flags.
- **Enhanced CLI**: Added `skrills validate` and `skrills analyze` commands for skill quality assurance.
- **MCP Tools**: Added `validate-skills` and `analyze-skills` tools to the MCP server.
- **Comprehensive Tests**: Added 53 tests for bidirectional skill sync covering validation, autofix, edge cases, and negative testing.
- **Architecture Pivot**: Removed redundant skill-serving functionality now that Claude Code and Codex CLI have native skill support. Skrills now focuses on validation, analysis, and sync.
- **REMOVED**: `autoload.rs`, `emit.rs`, and related autoload/emit functionality.
- **REMOVED**: CLI commands: `list`, `list-pinned`, `pin`, `unpin`, `auto-pin`, `history`, `emit-autoload`, `render-preview`.
- **REMOVED**: MCP tools: `list-skills`, `autoload-snippet`, `render-preview`, `runtime-status`, `set-runtime-options`, `pin-skills`, `unpin-skills`, `refresh-cache`.

## 0.3.0 - 2025-12-12
- **NEW: Subagents Module**: Added comprehensive subagent functionality with MCP server support. Run subagents via `list-subagents`, `run-subagent`, and `get-run-status` tools.
- **NEW: Backend Support**: Implemented dual backend support for both Claude-style and Codex-style subagent execution with configurable adapters.
- **NEW: Sync Infrastructure**: Introduced cross-agent sync orchestration with `SyncOrchestrator`, `ClaudeAdapter`, and `CodexAdapter` for multi-agent coordination.
- **Documentation**: Added comprehensive AGENTS.md (1500+ lines) with subagent usage examples, configuration options, and best practices.
- **Enhanced CLI**: Added sync commands (`skrills sync import`, `skrills sync export`, `skrills sync report`) for cross-agent skill synchronization.
- **Testing**: Added end-to-end integration tests for subagents functionality ensuring reliable operation across different backends.
- **BREAKING**: Removed the gateway crate and related functionality. The gateway approach has been replaced with a simpler MCP server integration for Codex.
- Codex setup now uses AGENTS.md instructions combined with MCP server registration in `~/.codex/config.toml`.
- Setup no longer generates TLS certificates or wrapper scripts; it now directly registers the MCP server.
- Removed gateway-related MCP tools (`gateway-status`, `gateway-start`, `gateway-restart`, `gateway-stop`).
- The MCP server provides full skill management capabilities including two-tier caching (discovery + content), trigram-based semantic matching, auto-pinning, and usage history tracking.

## 0.2.2 - 2025-12-04
- **Focus on Claude Code**: Simplified integration to focus on Claude Code hook-based skill injection.
- The installer now accepts `--client claude` flag (or `SKRILLS_CLIENT` environment variable) to target specific hook and configuration paths.
- Aligned all workspace crates to version 0.3.0 and updated `book/src/release.md` with the latest release notes.
- A security vulnerability was addressed by updating `rmcp` from 0.9.1 to 0.10.0. This change replaces the unmaintained `paste` dependency (version 1.0.15) with the actively maintained `pastey` (version 0.2.0).
- Autoload now uses `SKRILLS_EMBED_THRESHOLD` as the default embedding threshold if the command-line flag is not set. Manifest previews are now scaled to byte budgets to prevent missed filters and mismatches in gzip previews.
- The skills manifest loader now supports both the legacy array-only JSON format and the new structured object format. This ensures backward compatibility and prevents parsing failures with older configurations.

## 0.2.0 - 2025-11-26
- **Project Restructuring**: The project has been reorganized from a monolithic structure to a modular one, improving maintainability and scalability.
- **Project Renaming**: The project has been renamed from "codex-mcp-skills" to "skrills" across all documentation and source code.
- **Modular Workspace**: A new workspace structure has been implemented, with distinct crates for `discovery`, `state`, and `server` functionalities.
- **CI/CD Enhancements**: The continuous integration pipeline now includes a code coverage workflow and checks for differences in the public API to prevent unintended breaking changes.
- **Documentation Overhaul**: All project documentation has been updated with improved formatting and now includes Mermaid diagrams for better visualization of system architecture.
- **Code Cleanup**: Obsolete binaries and artifacts have been removed to improve the overall maintainability of the codebase.

## 0.2.1 - 2025-11-26
- Added cargo publishing infrastructure with dependency validation and dry-run checks.
- Enhanced release workflow with automated crate publishing to crates.io.
- Improved test isolation in server module to prevent cross-test contamination.
- Updated project documentation with better formatting and clearer explanations.
- Added dependency order validation before packaging to ensure publishing reliability.
- Fixed Python syntax errors in release workflow and improved code formatting.
- Enhanced project icon and improved README documentation clarity.

## 0.1.14 - 2025-11-25
- Added `skrills doctor` to inspect Codex MCP config (`mcp_servers.json`, `config.toml`) and verify `type = "stdio"` and binary paths.
- Improved robustness of MCP tool schemas to always include `type = "object"`, preventing Codex from raising `missing field "type"` on startup.
- `serve` command gains `--trace-wire` / `SKRILLS_TRACE_WIRE=1` to log MCP initialize traffic; warm-up now defers until after handshake.
- Installers write `type = "stdio"` to both Codex config files, tolerate permission failures, add a POSIX `--local` build flag, and maintain PowerShell parity.

## 0.1.13 - 2025-11-25
- The installer filters release archives more strictly and uses a source build as a secondary option when no match is found.
- CI jobs are restricted by path changes to avoid unnecessary runs.

## 0.1.12 - 2025-11-25
- Improved installer release-asset lookup for resiliency across different GitHub API responses.

## 0.1.11 - 2025-11-25
- Release workflow skips asset uploads if a release with the same tag already exists, preventing duplicate publishes.

## 0.1.10 - 2025-11-25
- Release workflow creates the GitHub release before uploading assets to eliminate race conditions.

## 0.1.9 - 2025-11-25
- Fixed release upload include patterns to ensure platform archives are attached correctly.

## 0.1.8 - 2025-11-25
- The manifest flag is now respected when hiding agent documentation in autoload outputs.
- Cached cargo artifacts in the audit workflow to speed up security checks.
- Corrected release include paths across workflows.

## 0.1.7 - 2025-11-25
- Added release dry-run builds and cache reuse in CI to validate artifacts before tagging.

## 0.1.6 - 2025-11-24
- Switched to supported archive options in the release workflow for better cross-platform packaging.

## 0.1.5 - 2025-11-24
- Set the `ZIP` flag in the Windows upload step to produce valid Windows artifacts.

## 0.1.4 - 2025-11-24
- Fixed CI upload action inputs for Rust binaries.
- Stabilized core tests by fixing race conditions in environment variable handling.

## 0.1.3 - 2025-11-24
- The installer builds from source in an isolated cargo home as a secondary option when release assets are missing.
- Asset selection prioritizes `jq`, with `awk` as a secondary option, and includes optional `pipefail` for POSIX shells.
- Release workflow asset names use the action template placeholder (`{{ target }}`) to publish platform tarballs reliably.

## 0.1.2 - 2025-11-24
- Added project icon at `assets/icon.png` and linked it in the README.
- Installer docs use branch-agnostic `/HEAD/` URLs.
- Makefile `book` target builds and opens the mdBook.
- Added comparison and FAQ chapters to the book.
- Release workflow assets now interpolate the `target` correctly; the audit workflow runs `cargo audit` directly.

## 0.1.1 - 2025-11-24
- Added a single-command installer that registers the Codex hook/MCP server by default.
- Introduced mdBook documentation and a GitHub Pages deployment workflow.
- Added a CI workflow (fmt, clippy, tests, docs, mdbook) and refined release asset naming.
- Expanded Makefile with docs/book targets and CLI demos.
- Documented child-process safety, cache TTL manifest option, and installer defaults.

## 0.1.0 - 2025-11-24
- Split the project into a Rust workspace: `crates/core` (library/MCP server) and `crates/cli` (binary wrapper).
- Added structured `_meta` outputs across tools with priority ranks and duplicate info.
- Synced `AGENTS.md` generation to include per-skill `priority_rank` and an overall priority list.
- Enhanced README with clearer installation/usage, universal sync, TUI, and structured output examples.
