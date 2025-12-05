# Changelog Highlights

## 0.3.0 (2025-12-02)
- **Architecture**: Implemented a gateway middleware server, with HTTP proxy capabilities.
- **New Crates**: Introduced a `gateway` crate, with features such as TLS support, rate limiting, and comprehensive metrics collection.
- **Security**: Enhanced security by adding a threat model, implementing security headers, and integrating mutual TLS (mTLS) authentication.
- **Performance**: Optimized performance of caching mechanisms, embedding generation, and efficient MCP token usage.
- **Testing**: Expanded test coverage with new integration tests, error recovery tests, and transport layer validation.
- **Documentation**: Updated the book with new sections on gateway functionality, security, performance, and monitoring.
- **CLI**: Extended CLI capabilities with a new gateway command, improved diagnostics, and introduced new runtime status tools.
- **Security Fix**: Addressed a security vulnerability by updating `rmcp` from version 0.9.1 to 0.10.0, which replaces the unmaintained `paste` (1.0.15) dependency with the actively maintained `pastey` (0.2.0).

## 0.2.1 (2025-11-26)
- **Publishing**: Implemented a new Cargo publishing workflow that includes dependency validation and dry-run checks.
- **Release**: Enhanced the release workflow with automated publishing of crates to crates.io.
- **Testing**: Improved test isolation within the server module to prevent cross-test contamination.
- **Documentation**: Updated project documentation with improved formatting.
- **Build**: Added dependency order validation before packaging, to ensure reliable publishing.
- **Workflows**: Corrected Python syntax errors in the release workflow and improved code formatting.
- **Visuals**: Updated the project icon and README documentation.

## 0.2.0 (2025-11-26)
- **Refactoring**: Reorganized the project from a monolithic structure to a more modular architecture.
- **Renaming**: Renamed the project from "codex-mcp-skills" to "skrills" across all documentation and source code.
- **Modular Architecture**: Implemented a new workspace structure, with `discovery`, `state`, and `server` crates.
- **CI/CD**: Added a code coverage workflow and implemented checks for public API changes.
- **Documentation**: Updated all documentation with improved formatting and added Mermaid diagrams.
- **Legacy Cleanup**: Removed obsolete binaries and artifacts to improve maintainability.

## 0.1.14
- Introduced `doctor` diagnostics, `--trace-wire` logging, and enhanced schema enforcement (specifically for `type = "object"`).
- Installers now enforce `type = "stdio"` within Codex configurations and provide support for `--local` builds.

## 0.1.13
- The installer now filters archives more rigorously and utilizes source builds as a secondary option.
- CI jobs are now configured to execute based on relevant path changes.

## 0.1.12
- Improved the release asset lookup mechanism within installers for enhanced reliability.

## 0.1.11
- The release workflow now skips asset uploads if a release with the same tag already exists, preventing duplication.

## 0.1.10
- The GitHub release is now created prior to uploading assets, effectively preventing race conditions.

## 0.1.9
- Fixed release upload include patterns to ensure the correct attachment of platform-specific archives.

## 0.1.8
- The manifest flag is now respected when suppressing agent documentation.
- Cached the audit workflow and corrected release include paths.

## 0.1.7
- Introduced release dry-run builds and implemented cache reuse within the CI pipeline.

## 0.1.6
- Switched to supported archive options in the release workflow to improve cross-platform packaging compatibility.

## 0.1.5
- Activated the ZIP flag in the Windows upload step to ensure the generation of valid artifacts.

## 0.1.4
- Fixed CI upload inputs specifically for Rust binaries and stabilized environment-driven tests.

## 0.1.3
- The installer now builds from source within a temporary Cargo home environment if a release asset is unavailable.
- Asset selection prioritizes `jq`, utilizing `awk` as a secondary option; `pipefail` remains optional.
- Release assets now employ the action `{{ target }}` placeholder to ensure correct platform-specific tarball generation.

## 0.1.2
- Added a project icon (located at `assets/icon.png`) and linked it within the README documentation.
- Installer documentation now utilizes branch-agnostic `/HEAD/` URLs. Updates were also made to the README, book, and FAQ sections.
- The Makefile `book` target now builds and automatically opens the mdBook documentation.
- Added new comparison and FAQ chapters to the book, and integrated an extended FAQ section into `docs/`.
- Release workflow assets now correctly interpolate the target, and the audit workflow directly executes `cargo audit`.

## 0.1.1
- The single-command installer now configures Codex hook and MCP registration by default, with an opt-out option available via `SKRILLS_NO_HOOK`.
- Integrated mdBook documentation with GitHub Pages deployment. The Makefile now includes new targets for book and documentation builds, as well as CLI demos.
- The CI workflow executes `fmt`, `clippy`, tests, and `mdbook`. Release assets are named `skrills-<target>.tar.gz`.
- Documentation has been expanded to cover child-process safety, the cache TTL manifest option, and installer default configurations.

## 0.1.0
- Initiated a workspace split, separating `crates/core` (comprising the MCP server and library) and `crates/cli` (the binary).
- Structured `_meta` fields across various tools to include priority ranks and duplicate information handling.
- `AGENTS.md` synchronization now incorporates per-skill priority ranks and an overall priority list.
- Refreshed the README documentation to include updated installation and usage instructions, universal synchronization details, TUI information, and examples of structured outputs.

For complete details, please refer to the [full changelog in the repository](docs/CHANGELOG.md).