# Release and Distribution Process

This document outlines the process for releasing and distributing `skrills`.

## Build Targets

`skrills` is built for the following target architectures:

-   `x86_64-unknown-linux-gnu`
-   `aarch64-unknown-linux-gnu`
-   `x86_64-apple-darwin`
-   `aarch64-apple-darwin`
-   `x86_64-pc-windows-msvc`
-   `aarch64-pc-windows-msvc`

## Asset Naming Convention

Release assets follow the naming convention `skrills-<target>.tar.gz`. Inside each archive, the binary is placed in the root and is named `skrills` (or `skrills.exe` for Windows builds).

## Installers

Our `curl` (for macOS/Linux) and `PowerShell` (for Windows) installation scripts automatically select the correct release asset using the GitHub API. These installers also register the MCP server by adding `type = "stdio"` to both `~/.codex/mcp_servers.json` and `~/.codex/config.toml`. The default GitHub repository is `athola/skrills`, but this can be overridden by setting the `SKRILLS_GH_REPO` environment variable.

## Continuous Integration (CI)

GitHub Actions are configured to build and upload release assets when `v*` tags are pushed. The `mdBook` documentation is also automatically deployed to GitHub Pages as part of the CI pipeline.

### crates.io publishing
- Crates publish in dependency order: `skrills-state`, `skrills-discovery`, `skrills-server`, `skrills`.
- Releases require `CARGO_REGISTRY_TOKEN` in repository secrets. The release workflow now preflights the token and performs `cargo publish --dry-run` for all crates before uploading.
- Pull requests touching Cargo/workflow files run a dry-run publish job to catch failures early.

## Documentation

-   **Rust Documentation**: Run `make docs` to generate and view the `cargo doc` documentation.
-   **Project Book**: Run `make book` to build the `mdBook` documentation and open it in your default browser. For live reloading during development, use `make book-serve`.

## Build Features

-   The `watch` feature, enabled by default, enables filesystem watching.
-   For minimal builds, use `--no-default-features` with `cargo build` or the `make build-min` command.

For maintainer notes on artifact layout, refer to `docs/release-artifacts.md`.

## Recent Releases

- **0.2.1 (unreleased)**: Version alignment for crates.io publishing (no functional changes).
- **0.2.0 (2025-11-26)**: Added crates.io publish automation (preflight token + dry-runs), deterministic embedding test overrides, and crates.io/book install docs.
- **0.1.14 (2025-11-25)**: Added `doctor` diagnostics, `--trace-wire` logging, schema hardening (`type = "object"`), and installers that write `type = "stdio"` to both Codex config files (plus `--local` build flag).
- **0.1.13 (2025-11-25)**: Installer filters release archives and uses source builds as a secondary option; CI jobs gate on relevant path changes.
- **0.1.12 (2025-11-25)**: More robust installer asset lookup.
- **0.1.11 (2025-11-25)**: Release workflow skips uploads when a release already exists.
- **0.1.10 (2025-11-25)**: Create the GitHub release before asset uploads to avoid races.
- **0.1.9 (2025-11-25)**: Fixed release upload include patterns for platform archives.
- **0.1.8 (2025-11-25)**: Respected manifest flag for hiding agents doc; cached audit workflow; corrected release include paths.
- **0.1.7 (2025-11-25)**: Added release dry-run builds and cache reuse in CI.
- **0.1.6 (2025-11-24)**: Switched to supported archive options in release packaging.
- **0.1.5 (2025-11-24)**: Set ZIP flag in Windows upload step.
- **0.1.4 (2025-11-24)**: Fixed CI upload inputs and stabilized env-driven tests.
