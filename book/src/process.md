# Development Process and Workflow Guidelines

This document outlines development processes and safety considerations to maintain a stable and reliable runtime environment.

## Child Process Management

Effective child process management is critical to prevent system instability, like resource leaks or unexpected process termination.

- On Unix-like systems, a `SIGCHLD` handler configured with `SA_NOCLDWAIT | SA_RESTART` is installed during startup. This prevents "zombie" processes.
- When spawning new child processes, you must manage their exit status without directly using `waitpid`, or temporarily override and restore the `SIGCHLD` handler.
- Non-Unix builds currently use a no-operation (no-op) stub. If platform-specific child process spawning capabilities are introduced, equivalent safeguards must be implemented.

## Workflow Best Practices

Follow these practices to maintain code quality and development standards:

- **Centralize Shared Logic**: All shared functionality and core logic must be in the [`crates/core`](crates/core) module. The [`crates/cli`](crates/cli) crate should be a thin wrapper, focusing on command-line interface concerns.
- **Changelog Updates**: Ensure the changelog is updated for all user-visible changes, including new CLI flags, changes to output formats, `AGENTS` sync behavior, and priority rule adjustments.
- **Pre-Publishing Checks**: Before publishing, run `cargo fmt` to ensure code formatting and `cargo test` to confirm test integrity.

## Development Checklist

Before finalizing and merging changes, consider the following checklist:

- Verify that no new zombie or unmanaged child processes are introduced.
- Confirm a changelog entry exists for all user-facing changes.
- Ensure all tests pass and code formatting adheres to standards.
- Confirm all new CLI flags are documented in both [`README.md`](README.md) and the project book.