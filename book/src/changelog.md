# Changelog Highlights

## 0.5.3 (2026-01-23)

- **Docs**: Streamlined README with navigation links and simplified installation section.
- **NEW: Config File Support**: Added `--config` flag and `skrills.toml` config file for persistent server settings.
- **NEW: TLS Auto-Generation**: Generate self-signed TLS certificates with `--tls-generate` for development HTTPS.

## 0.5.2 (2026-01-22)

- **NEW: HTTP Transport for MCP Servers**: Added support for HTTP-type MCP servers (like context7) which use `type="http"` with `url` and `headers` fields instead of `command/args/env`.
- **Dependency Cleanup**: Removed unused dependencies (`pastey`, `sha2`, `flate2`) from `crates/server`, `regex` from `crates/intelligence`, `anyhow` from `crates/analyze`, `thiserror` from `crates/sync` and `crates/subagents`.
- **Dead Code Removal**: Removed unused methods from `SkillCache` and unused imports from metrics and test modules.
- **Bug Fix**: Filter whitespace-only descriptions at extraction point (not just empty strings).
- **Improved**: Skill discovery summary log changed from debug to info level for better observability.
- **Testing**: Added 5 new tests for multi-word description matching (first/middle/last word, special characters, long descriptions).

## 0.5.1 (2026-01-21)

- **Refactor: Skill Trace Commands**: Split monolithic `/skill-trace` into three focused commands:
  - `/skill-trace-enable` - Enable tracing with instrumentation
  - `/skill-trace-disable` - Disable tracing and cleanup
  - `/skill-trace-status` - Check current tracing state
- **Refactor: Test Utils Crate**: Extracted shared test infrastructure into `crates/test-utils/`.
- **Code Quality**: Sorted command registrations alphabetically in plugin.json.
- **Docs**: Cleaned up `docs/audit-logging.md`.

## 0.5.0 (2026-01-20)

- **NEW: GitHub Copilot CLI Support**: Full bidirectional sync for `~/.copilot` (skills, MCP servers, preferences).
- **NEW: Agent Sync**: Sync Claude Code subagents to Copilot with format transformation (removes `model`/`color`, adds `target: github-copilot`).
- **NEW: CLI Flags**: Added `--from copilot` and `--to copilot` for sync commands.
- **NEW: MCP Tools**: `sync-from-copilot`, `sync-to-copilot`, all 6 source→target skill sync combinations.
- **NEW: Validation Target**: `--target copilot` for skill validation.
- **Testing**: 20 Copilot sync integration tests.

## 0.4.12 (2026-01-16)

- **Testing**: Added 59 new tests across tool_schemas (20), sync report (20), and validation common (19) modules.

## 0.4.11 (2026-01-15)

- **Refactor**: Improved test infrastructure with RAII guards (`EnvVarGuard`, `TestFixture`).
- **Bug Fix**: Logging for silent failure path in skill validation; `saturating_mul` for timestamp overflow.
- **Docs**: Consolidated 14 documentation files from bulleted lists to flowing prose.

## 0.4.10 (2026-01-15)

- **NEW: Confidence Type**: Type-safe `Confidence` newtype with clamping (0.0-1.0) for recommendations.
- **Improved: Observability**: Added tracing for behavioral and comparative recommendation modules.
- **NEW: Auto-Persist**: `--auto-persist` flag and `SKRILLS_AUTO_PERSIST` env var for automatic analytics caching.

## 0.4.9 (2026-01-12)

- **NEW: MCP Gateway**: Context-optimized tool loading via lazy schema loading.
- **NEW: Analytics Persistence**: Export/import analytics via `export-analytics` and `import-analytics`.
- **NEW: HTTP Security Options**: Added `--auth-token`, `--tls-cert`, `--tls-key`, `--cors-origins` flags.

## 0.4.8 (2026-01-10)

- **NEW: Skill Description Caching**: Optional `description` field in `SkillMeta` for richer fuzzy search.
- **Improved: Fuzzy Search**: Matches against descriptions in addition to names.

## 0.4.7 (2026-01-09)

- Version bump to republish after v0.4.6 tag protection issue.

## 0.4.6 (2026-01-08)

- **Bug Fixes**: Address 14 issues including warnings for skipped deps, YAML line/column info, SAFETY comments.
- **Testing**: Add 282 new tests across multiple modules.
- **Documentation**: Add audit-logging.md covering security events and SIEM integration.

## 0.4.5 (2026-01-03)

- **Testing**: Added comprehensive test coverage for tool handler functions.

## 0.4.4 (2026-01-02)

- **NEW: Empirical Skill Creation**: Generate skills from session patterns via `--method empirical`. Mines Claude Code/Codex CLI history for successful tool sequences and failure anti-patterns.
- **NEW: Comparative Recommendations**: Deviation scoring compares actual vs expected outcomes per skill category (Testing, Debugging, Documentation, etc.).
- **NEW: Behavioral Analytics**: Extract tool calls, file access patterns, and session outcomes from history.
- **Improved: Install Script**: User-scoped MCP registration (`--scope user`) with better error capture.

## 0.4.3 (2025-12-31)

- **NEW: HTTP Transport**: `--http` flag for HTTP transport instead of stdio.
- **Runtime Configuration**: HTTP flag is runtime-checked, not compile-time gated.
- **Default**: HTTP transport enabled by default in release builds.

## 0.4.2 (2025-12-29)

- **NEW: Fuzzy Skill Search**: Added `search-skills-fuzzy` MCP tool with trigram-based similarity matching for typo-tolerant skill discovery.
- **Example**: Query "databas" will find skills named "database" with high similarity scores.
- **Parameters**: `query` (search term), `threshold` (similarity cutoff, default 0.3), `limit` (max results).

## 0.4.1 (2025-12-27)

- **Hashing**: Switched file hash algorithm to blake2b-256 for improved performance and security.
- **CI/CD**: Improved error handling in publish verification workflow.
- **Documentation**: Added README for the intelligence crate.

## 0.4.0 (2025-12-24)

- **NEW: Intelligent Skills**: Added `skrills-intelligence` crate with project context analysis, skill recommendations, and skill creation.
- **NEW: Project Context Detection**: Automatically detect languages, frameworks, and project type from README and git history.
- **NEW: Smart Recommendations**: Context-aware skill recommendations based on project profile and usage analytics.
- **NEW: Skill Creation**: Create new skills via GitHub search or LLM generation with project context.
- **NEW: Usage Analytics**: Parse Claude Code and Codex CLI history for skill usage patterns.
- **Security**: Path traversal guard, GitHub query injection sanitization, CLI binary name validation.

## 0.3.5 (2025-12-21)

- **NEW: Agent Discovery**: Added `list-agents` MCP tool for discovering available agents with metadata caching.
- **NEW: Run Events Polling**: Added `get-run-events` MCP tool for polling-based event retrieval.
- **NEW: Codex CLI Adapter**: Subprocess execution of Codex CLI agents via `CodexCliAdapter`.
- **NEW: Smart Routing**: Automatic backend selection based on `agent_id` parameter.
- **NEW: Model Mapping**: Cross-platform model preference sync for consistent agent configuration.
- **Error Handling**: Structured `CliError` enum with better error messages; logging for silent failures.
- **Testing**: Comprehensive integration tests for subagent workflow.

## 0.3.4 (2025-12-19)

- **NEW: Skill Recommendations**: Added `skrills recommend` CLI command and `recommend-skills` MCP tool for suggesting related skills based on dependency graph relationships (dependencies, dependents, and siblings).
- **Options**: `--limit` for max recommendations, `--include-quality` for quality scores, `--format` for text/json output.

## 0.3.3 (2025-12-18)

- **NEW: Metrics Command**: Added `skrills metrics` CLI command and `skill-metrics` MCP tool for aggregate statistics including quality distribution, dependency graphs, and token usage.
- **NEW: Makefile Targets**: Added `make status`, `make install`, `make test-coverage`, `make security`, and `make deps-update` for developer workflows.
- **Dependency Graph**: Metrics include hub skill detection and orphan count from the dependency analysis.

## 0.3.2 (2025-12-17)

- **NEW: Dependency Resolution**: Skill dependency tracking via YAML frontmatter with semver constraints, circular dependency detection, and source pinning.
- **NEW: Skill Loading Trace**: Diagnostic tools for debugging skill loading (`skill-loading-status`, `skill-loading-selftest`, trace enable/disable).
- **Dependency Syntax**: Simple, structured, and compact syntax forms for declaring dependencies.
- **Optional Dependencies**: Configurable behavior for optional dependencies.
- **Extended Validation**: Dependency-related validation issues in the validate crate.
- **BREAKING**: rmcp updated to 0.10 (removed deprecated `info()` method).
- **BREAKING**: `SkillSource` enum now requires wildcard pattern matching.

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
