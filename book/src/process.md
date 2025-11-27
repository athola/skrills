# Development Process and Safety Guidelines

This document outlines the development processes and safety considerations for the project, aimed at ensuring a stable and predictable runtime environment.

## Child Process Management

Managing child processes correctly is essential to prevent system instability, such as resource leaks or unexpected termination.

-   On Unix-like systems, a `SIGCHLD` handler with `SA_NOCLDWAIT | SA_RESTART` is installed at startup to prevent "zombie" processes.
-   If you need to spawn child processes, ensure you either manage their exit status without `waitpid` or temporarily override and restore the `SIGCHLD` handler.
-   Non-Unix builds currently use a no-op stub for this handler. Before adding platform-specific child process spawning, implement equivalent safeguards for those platforms.

## Workflow Best Practices

Adhering to these practices ensures code quality and consistency:

-   **Centralize Shared Logic**: All shared functionality must be located in `crates/core`. The `crates/cli` must remain a thin wrapper.
-   **Changelog Updates**: Update the changelog for all user-visible changes, including new CLI flags, output format modifications, `AGENTS` synchronization behavior changes, and priority rule adjustments.
-   **Pre-Publishing Checks**: Before publishing or releasing any new version, always run `cargo fmt` and `cargo test` to confirm code formatting and test integrity.

## Development Checklist

Before finalizing changes, consider the following:

-   Ensure no new zombie processes or unmanaged child processes are introduced.
-   Verify that a changelog entry exists for all user-facing modifications.
-   Confirm that all tests pass and code formatting adheres to standards.
-   Ensure all new CLI flags are documented in both the `README.md` and the project book.
