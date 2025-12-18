# Changelog Highlights

## 0.3.3 (2025-12-18)

- **NEW: Metrics Command**: Added `skrills metrics` CLI command and `skill-metrics` MCP tool for aggregate statistics including quality distribution, dependency graphs, and token usage.
- **NEW: Makefile Targets**: Added `make status`, `make install`, `make test-coverage`, `make security`, and `make deps-update` for developer workflows.
- **Dependency Graph**: Metrics include hub skill detection and orphan count from the dependency analysis.

## 0.3.2 (2025-12-15)

- **NEW: Dependency Resolution System**: Added comprehensive skill dependency analysis and resolution.
- **Enhanced Analysis**: Improved token counting accuracy and added dependency graph visualization.
- **Performance Optimizations**: Optimized skill discovery and caching mechanisms.
- **Documentation Updates**: Added detailed dependency resolution documentation and ADR.

## 0.3.1 (2025-12-13)

- **NEW: Validation Crate** (`skrills-validate`): Validate skills for Claude Code (permissive) and Codex CLI (strict frontmatter requirements). Includes auto-fix capability to add missing frontmatter.
- **NEW: Analysis Crate** (`skrills-analyze`): Token counting, dependency analysis, and optimization suggestions for skills.
- **NEW: CLI Commands**: Added `skrills validate` and `skrills analyze` commands for skill quality assurance.
- **NEW: MCP Tools**: Added `validate-skills` and `analyze-skills` tools to the MCP server.
- **Architecture Pivot**: Removed redundant skill-serving functionality now that Claude Code and Codex CLI have native skill support. Skrills now focuses on validation, analysis, and sync.
- **Comprehensive Tests**: Added 53 tests for bidirectional skill sync.
- **REMOVED**: Autoload functionality (`autoload.rs`, `emit.rs`).
- **REMOVED**: CLI commands: `list`, `list-pinned`, `pin`, `unpin`, `auto-pin`, `history`, `emit-autoload`, `render-preview`.
- **REMOVED**: MCP tools: `list-skills`, `autoload-snippet`, `render-preview`, `runtime-status`, `set-runtime-options`, `pin-skills`, `unpin-skills`, `refresh-cache`.

## 0.3.0 (2025-12-12)

- **NEW: Subagents Module**: Comprehensive subagent functionality with MCP server support via `list-subagents`, `run-subagent`, and `get-run-status` tools.
- **NEW: Backend Support**: Dual backend support for both Claude-style and Codex-style subagent execution.
- **NEW: Sync Infrastructure**: Cross-agent sync orchestration with `SyncOrchestrator` and adapters for Claude/Codex.
- **Documentation**: Added comprehensive AGENTS.md with subagent usage examples.
- **BREAKING**: Removed the gateway crate and related functionality. Replaced with simpler MCP server integration.
- **Security Fix**: Updated `rmcp` from 0.9.1 to 0.10.0, replacing unmaintained `paste` with `pastey`.

## 0.2.2 (2025-12-04)

- **Focus on Claude Code**: Simplified integration to focus on Claude Code hook-based skill injection.
- **Installer Improvements**: Added `--client claude` flag and `SKRILLS_CLIENT` environment variable.
- Aligned workspace crates to version 0.3.0.

## 0.2.1 (2025-11-26)

- **Publishing**: Cargo publishing workflow with dependency validation and dry-run checks.
- **Release**: Automated crate publishing to crates.io.
- **Testing**: Improved test isolation in server module.
- **Documentation**: Updated formatting and clarity.

## 0.2.0 (2025-11-26)

- **Refactoring**: Reorganized from monolithic to modular architecture.
- **Renaming**: Project renamed from "codex-mcp-skills" to "skrills".
- **Modular Architecture**: New workspace with `discovery`, `state`, and `server` crates.
- **CI/CD**: Added code coverage workflow and public API change checks.
- **Documentation**: Added Mermaid diagrams for architecture visualization.

## 0.1.x Releases

See [full changelog](https://github.com/athola/skrills/blob/master/docs/CHANGELOG.md) for details on earlier releases including:
- 0.1.14: Doctor diagnostics, `--trace-wire` logging
- 0.1.13: Installer archive filtering improvements
- 0.1.12-0.1.0: Initial releases with installer, mdBook, and CI/CD setup
