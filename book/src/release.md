# Release Artifacts and Distribution

This document provides an overview of the release and distribution process for the `skrills` project.

## Build Targets

`skrills` is built to support the following target architectures:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

## Asset Naming Convention

Release assets follow the naming convention `skrills-<target>.tar.gz`. The executable binary is at the root of the archive and is named `skrills` (or `skrills.exe` for Windows builds).

## Installers

The provided `curl` (for macOS/Linux) and `PowerShell` (for Windows) installation scripts automatically select the correct release asset by querying the GitHub API. These installers also register the MCP server by adding `type = "stdio"` to both `~/.codex/mcp_servers.json` and `~/.codex/config.toml`. The default GitHub repository is `athola/skrills`, but this can be overridden by setting the `SKRILLS_GH_REPO` environment variable.

## Continuous Integration (CI)

GitHub Actions are configured to build and upload release assets when `v*` tags are pushed. At the same time, the `mdBook` documentation is automatically deployed to GitHub Pages in the CI pipeline.

### crates.io Publishing
- Crates are published to `crates.io` in a defined dependency order: `skrills-state`, `skrills-discovery`, `skrills-server`, and `skrills`.
- Releases require the `CARGO_REGISTRY_TOKEN` to be configured in repository secrets. The release workflow validates this token and runs `cargo publish --dry-run` for all crates before actual publication.
- Pull requests that modify Cargo or workflow files automatically trigger a dry-run publish job. This helps catch potential publishing failures early.

## Documentation

- **Rust Documentation**: Run `make docs` to generate and view the `cargo doc` documentation.
- **Project Book**: Run `make book` to build and open the `mdBook` documentation. For live reloading during development, use `make book-serve`.

## Build Features

- The `watch` feature, which provides filesystem watching, is enabled by default.
- For minimal builds, run `cargo build` with the `--no-default-features` flag or use `make build-min`.

Maintainer notes regarding artifact layout can be found in [`docs/release-artifacts.md`](docs/release-artifacts.md).

## Recent Releases

- **0.3.0 (2025-12-04)**: Refactored for Claude Code hook-based integration. Improved MCP server stability, skill discovery, and documentation.
- **0.2.1 (2025-11-28)**: Added crates.io publishing automation with dependency validation.
- **0.2.0 (2025-11-26)**: Implemented `crates.io` publishing automation (with token validation and dry-runs), introduced deterministic embedding test overrides, and updated installation documentation.
- **0.1.14 (2025-11-25)**: Added `doctor` diagnostics, `--trace-wire` logging, schema hardening (for `type = "object"`), and updated installers to write `type = "stdio"` to both Codex configuration files (with a `--local` build flag).
- **0.1.13 (2025-11-25)**: The installer filters release archives more rigorously and uses source builds as a secondary option. CI jobs trigger based on relevant path changes.
- **0.1.12 (2025-11-25)**: Improved installer asset lookup robustness.
- **0.1.11 (2025-11-25)**: The release workflow now skips asset uploads if a release with the same tag already exists.
- **0.1.10 (2025-11-25)**: The GitHub release is created before asset uploads to prevent race conditions.
- **0.1.9 (2025-11-25)**: Corrected release upload include patterns for proper handling of platform archives.
- **0.1.8 (2025-11-25)**: Manifest flag respected for hiding agent documentation. Audit workflow cached, release include paths corrected.
- **0.1.7 (2025-11-25)**: Introduced release dry-run builds and implemented cache reuse in the CI pipeline.
- **0.1.6 (2025-11-24)**: Switched to supported archive options in the release packaging process.
- **0.1.5 (2025-11-24)**: ZIP flag set in Windows upload step for correct artifact generation.
- **0.1.4 (2025-11-24)**: Fixed CI upload inputs and stabilized environment-driven tests.