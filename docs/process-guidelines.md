# Process and Workflow Guidelines

This document outlines guidelines for process management and development workflow to ensure a stable and predictable runtime environment.

## Child Process Management

Effective child process management is critical to preventing system instability, which can lead to resource leaks or the accumulation of "zombie" processes.

- On Unix-like operating systems, a `SIGCHLD` handler, configured with `SA_NOCLDWAIT | SA_RESTART`, is installed during system startup. This handler automatically cleans up unexpected subprocesses.
- If you add new child processes on Unix systems, you must ensure their compatibility with this policy. This can be done by either collecting the exit status without directly using `waitpid` or by temporarily overriding the signal disposition before spawning the process.
- For non-Unix builds, a no-operation (no-op) stub is currently in place. If platform-specific process management capabilities are extended to other target environments, you must implement equivalent safeguards.

## Workflow Best Practices

- **Centralize Shared Logic**: All shared functionality and core logic must be centralized within the [`crates/core`](crates/core) module. The [`crates/cli`](crates/cli) crate should be a thin wrapper, focusing on command-line interface concerns and interactions.
- **Update the Changelog**: Ensure that [`docs/CHANGELOG.md`](docs/CHANGELOG.md) is updated to reflect all user-visible changes. This includes, but is not limited to, the introduction of new CLI flags, modifications to output formats, or changes to priority rules.
- **Run Pre-Publish Checks**: Before publishing, you must run `cargo fmt` for code formatting and `cargo test` to run tests. These steps ensure code quality and functional correctness.

## MCP Dependency Strategy

- **`rmcp`**: The `rmcp` crate is a workspace dependency, declared in the root [`Cargo.toml`](Cargo.toml) file. Individual crates should only enable the `rmcp` features they require.
- **`pastey`**: `pastey` is a transitive-only dependency, used by `rmcp` versions 0.10.0 and above to support procedural macros. It serves as a replacement for the unmaintained `paste` crate. Do not add `pastey` as a direct dependency.
- **Updates**: When updating `rmcp`, ensure the workspace version is updated once, then run all MCP integration tests. The version of `pastey` should always match the `rmcp` version.
- **Security**: The `rmcp` crate should be considered supply-chain critical, so monitor it for security advisories. `pastey`, on the other hand, is a compile-time-only dependency with a negligible runtime footprint and is actively maintained.
