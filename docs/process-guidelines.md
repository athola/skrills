# Process and Workflow Guidelines

## Child Process Management

Proper process management prevents resource leaks and zombie processes.

- **Unix-like systems**: A `SIGCHLD` handler (`SA_NOCLDWAIT | SA_RESTART`) installs at startup to clean up subprocesses automatically.
- **New processes**: If you spawn new child processes on Unix, ensure compatibility by collecting exit status without blocking `waitpid`, or by overriding the signal disposition locally.
- **Non-Unix systems**: Current builds use a no-op stub. Implement equivalent safeguards if extending process management to other platforms.

## Workflow Best Practices

- **Centralize Shared Logic**: Place shared functionality in the appropriate library crates (e.g., `crates/server`, `crates/sync`). Keep `crates/cli` as a thin wrapper for command-line interactions.
- **Update the Changelog**: Record user-visible changes (new flags, output formats, priority rules) in `docs/CHANGELOG.md`.
- **Run Pre-Publish Checks**: Run `cargo fmt` and `cargo test` before publishing to ensure code quality.

## MCP Dependency Strategy

- **`rmcp`**: Defined as a workspace dependency in the root `Cargo.toml`. Enable only required features in individual crates.
- **`pastey`**: A transitive dependency used by `rmcp` (≥0.10.0) for procedural macros. Do not add it directly.
- **Updates**: Update the workspace version of `rmcp` once, then run all MCP integration tests. Ensure `pastey` matches the `rmcp` version.
- **Security**: Treat `rmcp` as supply-chain critical. Monitor it for advisories. `pastey` is compile-time only and low-risk.
