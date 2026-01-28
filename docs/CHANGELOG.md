# Changelog

## 0.5.6 - 2026-01-28
- **Testing**: Added BDD-style unit tests for skill management modules (deprecation, pre-commit, profiling, rollback, usage-report) covering serialization, YAML escaping, git log parsing, file filtering, percentage calculations, and version hash validation.

## 0.5.5 - 2026-01-26
- **NEW: TLS Certificate Management**: Added `skrills cert` subcommand with `status`, `renew`, and `install` operations for managing TLS certificates. Certificate validity is displayed on server startup.
- **Refactor: Copilot Adapter**: Split monolithic `copilot.rs` into focused modules (agents, commands, mcp, paths, preferences, skills, utils, tests) for improved maintainability and testability.
- **Testing**: Added unit tests for certificate parsing covering missing file handling, valid PEM parsing, and invalid PEM detection.

## 0.5.4 - 2026-01-25
- **Testing**: Added BDD-style tests for configuration loading (backend/config.rs) covering defaults, custom values, error handling, and edge cases
- **Testing**: Added directory validation tests (validate/lib.rs) for skill discovery, hidden directory handling, multi-target validation, and edge cases
- **Documentation**: Updated README to reflect 37 CLI commands and added skill management section
- **Internal**: Improved test coverage with 407 new lines of tests across core modules

## 0.5.3 - 2026-01-24
- **Bug Fix**: Fixed CLI binary selection in subagent service to use the `backend` parameter when spawning CLI subprocesses. Previously, running in Codex would incorrectly spawn the `claude` binary instead of `codex`, causing process spawn failures. (#133)
- **NEW: Config File Support**: Security and serve options can now be configured via `~/.skrills/config.toml`. Precedence: CLI > ENV > config file. (#103)
- **NEW: TLS Auto-Generation**: Added `--tls-auto` flag to auto-generate self-signed TLS certificates for development. Certificates are stored in `~/.skrills/tls/` and reused across server restarts. (#104)
- **NEW: Skill Management Commands**: Added 9 new CLI commands for skill lifecycle management:
  - `skill-deprecate`: Mark skills as deprecated with migration guidance
  - `skill-rollback`: Revert skills to previous git versions
  - `skill-profile`: View skill execution statistics from analytics cache
  - `skill-catalog`: Browse and search all available skills with filtering
  - `skill-import`: Import skills from local paths
  - `skill-usage-report`: Generate detailed usage reports
  - `skill-score`: Calculate quality scores for skills (0-100 scale)
  - `sync-pull`: Placeholder for future remote registry integration
  - `pre-commit-validate`: Validate skill files for git hooks
- **NEW: Module Files Sync**: Skills with companion files (helpers, configs) are now synced as complete units, enabling multi-file skill sync between agents.
- **NEW: Instructions Sync**: Sync instructions between Claude (`CLAUDE.md`) and Copilot (`*.instructions.md` format).
- **NEW: Codex Agent Support**: Claude agents are now synced to Codex as skills with "agent-" prefix until Codex adds native agent support.
- **Security**: Added git hash validation for rollback commands, config parse error logging, and proper error handling for silent failures.
- **Testing**: Added 37 new frontmatter tests and 6 skill management tests covering edge cases and error handling.
- **Increased max_depth**: Module file collection now traverses up to 10 levels deep (was 5) for deeply nested skill structures.

## 0.5.2 - 2026-01-22
- **NEW: HTTP Transport Support**: MCP servers using HTTP transport (type="http" with url/headers) are now properly synced. Fixes compatibility with servers like context7. (#111)
- **Bug Fix**: Filter whitespace-only descriptions at extraction point, not just empty strings. (#100)
- **Bug Fix**: Skill discovery summary now logs at info level instead of debug. (#101)
- **Testing**: Added tests for multi-word description matching and special characters. (#99)
- **Dependencies**: Pruned unused dependencies and dead code for smaller binary size.
- **CI**: Allow wildcard path deps in cargo-deny config.

## 0.5.1 - 2026-01-21
- **Refactor: Skill Trace Commands**: Split monolithic `/skill-trace` into three focused commands for clearer UX:
  - `/skill-trace-enable` - Enable tracing with instrumentation
  - `/skill-trace-disable` - Disable tracing and cleanup
  - `/skill-trace-status` - Check current tracing state
- **Refactor: Test Utils Crate**: Extracted shared test infrastructure (`TempEnv`, `TempSkillDir`, fixtures) into new `crates/test-utils/` crate for consistent test setup across all crates.
- **Code Quality**: Sorted command registrations alphabetically in plugin.json.
- **Docs**: Cleaned up `docs/audit-logging.md` - removed stale TODO links and parameterized version references.

## 0.5.0 - 2026-01-20
- **NEW: GitHub Copilot CLI Support**: Full bidirectional sync support for GitHub Copilot CLI (`~/.copilot`).
  - Skills: Sync between Copilot and Claude/Codex (SKILL.md format)
  - Agents: Sync from Claude plugins cache to `~/.copilot/agents/` with format transformation
  - MCP Servers: Read from/write to `mcp-config.json` (Copilot's separate MCP config file)
  - Preferences: Sync via `config.json` with security field preservation
  - Commands: Skipped (Copilot does not support slash commands - see FAQ)
- **NEW: Agent Sync**: Sync Claude Code subagents to Copilot with automatic format transformation (removes `model`/`color`, adds `target: github-copilot`).
- **NEW: CLI Flags**: Added `--from copilot` and `--to copilot` for sync-all and sync-* commands.
- **NEW: MCP Tools**: `sync-from-copilot`, `sync-to-copilot`, and enhanced `sync-skills` with all 6 source→target combinations.
- **NEW: Validation Target**: `--target copilot` for skill validation (same rules as Codex).
- **Testing**: Added 20 Copilot sync integration tests covering all sync directions.
- **Docs**: Added FAQ entry explaining why Copilot doesn't have slash commands.

## 0.4.12 - 2026-01-16
- **Testing**: Added 59 new tests across tool_schemas (20), sync report (20), and validation common (19) modules covering schema generation, report formatting, and validation issue handling.

## 0.4.11 - 2026-01-15
- **Refactor**: Improved test infrastructure with RAII guards (`EnvVarGuard`, `TestFixture`) for reliable parallel test execution.
- **Refactor**: Extracted API key error messages to constants in subagent backends.
- **Bug Fix**: Added logging for silent failure path in skill validation invariant checks.
- **Bug Fix**: Changed timestamp duration calculation to use `saturating_mul` to prevent integer overflow.
- **Docs**: Consolidated 14 documentation files from bulleted lists to flowing prose, reducing visual noise while preserving technical content.
- **Docs**: Added missing `skip_existing_commands` parameter to `sync_all_tool` documentation.
- **Testing**: Added 27 new tests covering cluster formation, deviation score boundaries, n-gram edge cases, behavioral patterns, and parse_trace_target case sensitivity.

## 0.4.10 - 2026-01-15
- **NEW: Confidence Type**: Added `Confidence` newtype with clamping constructor (0.0-1.0) for type-safe confidence scores in recommendations. (#73)
- **Improved: Type Safety**: Removed redundant `ClusteredBehavior::size` field in favor of computed method. (#74)
- **Improved: Observability**: Added tracing for silent early returns in behavioral and comparative recommendation modules. (#75)
- **NEW: Analytics Auto-Persistence**: Added `--auto-persist` flag to `recommend-skills-smart` command and `SKRILLS_AUTO_PERSIST` environment variable for automatic analytics caching after operations that build analytics. (#62)

## 0.4.9 - 2026-01-12
- **NEW: MCP Gateway**: Added context-optimized tool loading via lazy schema loading. Tools `list-mcp-tools`, `describe-mcp-tool`, and `get-context-stats` enable on-demand schema retrieval to reduce context window pressure.
- **NEW: Analytics Persistence**: Export and import usage analytics via `export-analytics` and `import-analytics` CLI commands. Analytics cache persists to `~/.skrills/analytics_cache.json`.
- **NEW: HTTP Security Options**: Added `--auth-token`, `--tls-cert`, `--tls-key`, and `--cors-origins` flags to `serve` command for production deployments.
- **Testing**: Added 11 new tests for analytics persistence functions and MCP gateway handlers.

## 0.4.8 - 2026-01-10
- **NEW: Skill Description Caching**: Added optional `description` field to `SkillMeta` for richer fuzzy skill search. Descriptions are extracted from YAML frontmatter during discovery.
- **Improved: Fuzzy Search**: Enhanced `search-skills-fuzzy` MCP tool to match against skill descriptions in addition to names.
- **Testing**: Added 7 new tests for description extraction covering various frontmatter formats and edge cases.
- **Code Quality**: Applied pedantic clippy auto-fixes (raw string hashes, redundant closures).

## 0.4.7 - 2026-01-09
- **Chore**: Version bump to republish after v0.4.6 tag protection issue.
- All changes from 0.4.6 are included in this release.

## 0.4.6 - 2026-01-08
- **Bug Fixes**: Address 14 issues including warnings for skipped optional dependencies, YAML line/column info in parse errors, SAFETY comments for regex patterns, and actionable hints for I/O errors.
- **Testing**: Add 282 new tests across claude_parser, codex_parser, context detector, github_search, dependencies, llm_generator, and scorer modules.
- **Documentation**: Add audit-logging.md covering security events, mTLS audit trails, and SIEM integration.
- **CI**: Revert sync crate name to skrills_sync (crates.io disallows renaming published crates).
- **Build**: Add fmt-check target to catch formatting issues in pre-commit hooks.

## 0.4.5 - 2026-01-03
- **Testing**: Added tests for tool handler functions including `parse_trace_target`, `skill_loading_status_tool`, `skill_loading_selftest_tool`, and `disable_skill_trace_tool`. Tests cover edge cases, dry-run modes, and target validation for Claude, Codex, and Both trace targets.

## 0.4.4 - 2026-01-02
- **NEW: Empirical Skill Creation**: Generate skills from observed session patterns via `--method empirical`. Clusters successful tool sequences and failure patterns from Claude Code/Codex CLI history.
- **NEW: Comparative Recommendations**: Deviation scoring compares actual skill-assisted outcomes against category baselines (Testing, Debugging, Documentation, etc.) to identify underperforming skills.
- **NEW: Behavioral Analytics**: Extract tool call sequences, file access patterns, and session outcomes (success/failure/partial) from session history for richer usage analysis.
- **Improved: Install Script**: Better error capture and user-scoped MCP registration with `--scope user` flag. Fallback logic now shows detailed error messages.

## 0.4.3 - 2025-12-31
- **NEW: HTTP Transport Installation**: Added `--http` flag to install scripts for HTTP transport instead of stdio. Installs systemd user service and configures MCP clients with HTTP URL.
- **Improved: Runtime HTTP Configuration**: The `--http` CLI flag is now always visible and runtime-checked, rather than compile-time gated.
- **Default: HTTP Transport Enabled**: HTTP transport feature is now enabled by default in release builds.

## 0.4.2 - 2025-12-29
- **NEW: Fuzzy Skill Search**: Added `search-skills-fuzzy` MCP tool with trigram-based similarity matching for typo-tolerant skill discovery (e.g., "databas" finds "database").
- **Improved Tests**: Added edge case tests for similarity matching (unicode, empty strings, punctuation, long strings) and integration tests for fuzzy search tool.

## 0.4.1 - 2025-12-27
- **Hashing**: Switched file hash algorithm to blake2b-256 for improved performance and security.
- **CI/CD**: Improved error handling in publish verification workflow.
- **Documentation**: Added README for the intelligence crate.

## 0.4.0 - 2025-12-24
- **NEW: Intelligent Skills**: Added `skrills-intelligence` crate with project context analysis, skill recommendations, and skill creation capabilities.
- **NEW: Project Context Detection**: Automatically detect programming languages, frameworks, project type, and extract keywords from README and git history.
- **NEW: Smart Recommendations**: Context-aware skill recommendations based on project profile, usage analytics, and dependency relationships.
- **NEW: Skill Creation**: Create new skills via GitHub search or LLM generation with project context awareness.
- **NEW: Usage Analytics**: Parse Claude Code and Codex CLI history for skill usage patterns and co-occurrence analysis.
- **Security Fixes**: Added path traversal guard, GitHub query injection sanitization, and CLI binary name validation.
- **Error Handling**: Improved error logging for silent failures and actionable GitHub API error messages.

## 0.3.5 - 2025-12-21
- **NEW: Agent Discovery**: Added `list-agents` MCP tool for discovering available agents with metadata and caching via `AgentRegistry`.
- **NEW: Run Events Polling**: Added `get-run-events` MCP tool for polling-based event retrieval during agent runs.
- **NEW: Codex CLI Adapter**: Added `CodexCliAdapter` for subprocess execution of Codex CLI agents.
- **NEW: Smart Routing**: Agent routing now automatically selects the appropriate backend based on `agent_id` parameter.
- **NEW: Model Mapping**: Added cross-platform model preference sync for consistent agent configuration.
- **Improved Error Handling**: Added structured `CliError` enum for CLI adapter with better error messages.
- **Logging**: Added error logging for silent failures to improve debugging.
- **Publishing**: Added missing crates.io metadata to workspace crates.
- **Testing**: Added integration tests for the subagent workflow.
- **Documentation**: Addressed PR review suggestions and added SAFETY comments for `json!()` expect invariants.

## 0.3.4 - 2025-12-19
- **NEW: Skill Recommendations**: Added `skrills recommend` CLI command and `recommend-skills` MCP tool for suggesting related skills based on dependency graph relationships (dependencies, dependents, and siblings).

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
- **Documentation**: Added ADR 0002 for dependency resolution architecture.
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
- **NEW: Subagents Module**: Added subagent functionality with MCP server support. Run subagents via `list-subagents`, `run-subagent`, and `get-run-status` tools.
- **NEW: Backend Support**: Implemented dual backend support for both Claude-style and Codex-style subagent execution with configurable adapters.
- **NEW: Sync Infrastructure**: Introduced cross-agent sync orchestration with `SyncOrchestrator`, `ClaudeAdapter`, and `CodexAdapter` for multi-agent coordination.
- **Documentation**: Added AGENTS.md (1500+ lines) with subagent usage examples, configuration options, and best practices.
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
