# Development Guide

This guide provides development information for contributors.

## Toolchain

- **Rust**: Ensure Rust 1.78+ is installed, preferably with `rustup`.
- **Formatting and Linting**: Use `cargo fmt` for formatting and `clippy` for linting to maintain code quality.
- **Documentation**: This project uses `mdbook` for documentation. Install it with `cargo install mdbook --locked`.

## Make Targets

The project's [`Makefile`](Makefile) offers several convenient targets to streamline development workflows:

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
make docs         # Builds the rustdoc API documentation
make book         # Compiles and automatically opens the mdBook
make book-serve   # Serves the mdBook with live-reloading capabilities, typically accessible at localhost:3000

# Cleaning up
make clean
make clean-demo
```

## Demo Sandbox

The `make demo-all` target creates a sandboxed environment for CLI testing. It builds a release binary, sets up a temporary home directory with a demo skill, and runs commands to validate end-to-end behavior without affecting your actual home directory.

## Testing

To run all tests, use:

```bash
cargo test --workspace --all-features
```

This command is aliased by `make test` and is part of the Continuous Integration (CI) pipeline, managed by `make ci`.

### Public API Guardrails

The `skrills-server` crate is currently in its pre-1.0 development phase. Refer to the SemVer guidance in [Public API and SemVer Policy](semver.md) and perform local public API checks before submitting changes. This ensures the API evolution policy is followed.

### Coverage

- **Local Coverage**: For local analysis, generate an HTML report with `cargo llvm-cov --workspace --html`, or an LCOV report for CI export with `cargo llvm-cov --workspace --lcov --output-path lcov.info`.
- **CI Coverage**: The [`coverage.yml`](.github/workflows/coverage.yml) workflow within our CI/CD pipeline runs the same coverage command and uploads the results to Codecov.