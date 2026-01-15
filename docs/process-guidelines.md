# Process and Workflow Guidelines

## Child Process Management

Proper process management prevents resource leaks and zombie processes. On Unix-like systems, a `SIGCHLD` handler (`SA_NOCLDWAIT | SA_RESTART`) installs at startup to clean up subprocesses automatically. If you spawn new child processes, ensure compatibility by collecting exit status without blocking `waitpid`, or by overriding the signal disposition locally. Non-Unix systems currently use a no-op stub, so implementing equivalent safeguards is necessary if extending process management to those platforms.

## Workflow Best Practices

Centralize shared logic in the appropriate library crates like `crates/server` or `crates/sync`, keeping `crates/cli` as a thin wrapper for command-line interactions. Record user-visible changes such as new flags or output formats in `docs/CHANGELOG.md`. Always run `cargo fmt` and `cargo test` before publishing to maintain code quality.

## MCP Dependency Strategy

We define `rmcp` as a workspace dependency in the root `Cargo.toml`, enabling only required features in individual crates. Note that `pastey` is a transitive dependency used by `rmcp` for procedural macros and should not be added directly. When updating, bump the workspace version of `rmcp` and run all MCP integration tests, ensuring `pastey` matches the version. Treat `rmcp` as supply-chain critical and monitor it for advisories.
