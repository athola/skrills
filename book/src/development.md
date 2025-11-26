# Development

This page provides information for developers who want to contribute to the project.

## Toolchain

-   **Rust**: Install version 1.78 or newer via `rustup`.
-   **Formatting and Linting**: Use `cargo fmt` and `clippy`.
-   **Documentation**: This book uses `mdbook`. Install it by running `cargo install mdbook --locked`.

## Make Targets

The `Makefile` provides several convenient targets:

```bash
# Code quality checks
make fmt
make lint
make check

# Testing
make test

# Building
make build
make build-min

# Running common commands
make serve-help
make emit-autoload

# End-to-end demo
make demo-all

# Documentation
make docs         # Build rustdoc documentation
make book         # Build and open the mdBook
make book-serve   # Serve the mdBook with live-reloading on localhost:3000

# Cleaning up
make clean
make clean-demo
```

## Demo Sandbox

The `make demo-all` target provides a sandboxed environment for testing the CLI. It builds a release binary, prepares a temporary home directory with a demo skill, and runs a series of commands (`list`, `pin`, `unpin`, `auto-pin`, `history`, `sync-agents`, `sync`, and `emit-autoload`) to validate the end-to-end behavior of the CLI without affecting your real home directory.

## Testing

To run the full test suite, use the following command:

```bash
cargo test --workspace --all-features
```

This is also aliased to `make test` and is run as part of the continuous integration pipeline (`make ci`).

### Public API guardrails

The `codex-mcp-skills-server` crate is pre-1.0. Follow the SemVer guidance in
[Public API & SemVer](semver.md) and run the public API check locally before
submitting changes.

### Coverage

- Local: `cargo llvm-cov --workspace --html` for an HTML report, or `cargo llvm-cov --workspace --lcov --output-path lcov.info` for CI export.
- CI: `.github/workflows/coverage.yml` runs the same command and uploads to Codecov.
