# Development

This page provides information for developers who want to contribute to the project.

## Toolchain

-   **Rust**: Version 1.78 or newer (installed via `rustup`).
-   **Formatting and Linting**: `cargo fmt` and `clippy` are used.
-   **Documentation**: This book is built with `mdbook`. You can install it by running `cargo install mdbook --locked`.

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
