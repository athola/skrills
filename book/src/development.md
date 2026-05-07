# Development Guide

This guide provides development information for contributors.

## Toolchain

- **Rust**: Install Rust 1.78+, preferably with `rustup`.
- **Formatting and Linting**: Use `cargo fmt` for formatting and `clippy` for linting to maintain code quality.
- **Documentation**: This project uses `mdbook` for documentation. Install it with `cargo install mdbook --locked`.

## Make Targets

The project's [`Makefile`](Makefile) provides targets for development workflows:

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

# Documentation
make docs         # Builds the rustdoc API documentation
make book         # Compiles and automatically opens the mdBook
make book-serve   # Serves the mdBook at localhost:3000 with live-reloading

# Cleaning up
make clean
```

## Demo Sandbox

The `make demo-all` target creates a sandboxed environment for CLI testing. It builds a release binary, sets up a temporary home directory with a demo skill, and runs commands to validate end-to-end behavior without affecting your actual home directory.

## Testing

To run all tests, use:

```bash
cargo test --workspace --all-features
```

This command is aliased by `make test` and is part of the Continuous Integration (CI) pipeline, managed by `make ci`.

### Release-Consistency Checks

Before tagging a release, run `make release-consistency` to verify
five parity invariants across the workspace:

1. All `crates/*/Cargo.toml` versions agree.
2. `plugins/skrills/.claude-plugin/plugin.json` version matches the
   workspace crate version.
3. Every command path in `plugin.json.commands` exists on disk.
4. The top-level `plugins/skrills/commands/*.md` count equals
   `plugin.json.commands.length`.
5. `.claude-plugin/marketplace.json` plugin entries agree with the
   workspace and each entry's `source` path exists on disk.

For a live inventory before running the parity tests, use
`make demo-release-consistency` — it prints the actual on-disk
versions and command counts before invoking the test suite.

The same tests are picked up by `make test` and `make test-integration`
via the `--test '*'` filter, so CI also enforces the invariants.

### Public API Guardrails

The `skrills-server` crate is currently in its pre-1.0 development phase. Refer to the SemVer guidance in [Public API and SemVer Policy](semver.md) and perform local public API checks before submitting changes. This verifies compliance with the API evolution policy.

### Coverage

- **Local Coverage**: For local analysis, generate an HTML report with `cargo llvm-cov --workspace --html`, or an LCOV report for CI export with `cargo llvm-cov --workspace --lcov --output-path lcov.info`.
- **CI Coverage**: The [`coverage.yml`](.github/workflows/coverage.yml) workflow within our CI/CD pipeline runs the same coverage command and uploads the results to Codecov.