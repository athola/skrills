# Process Handling and Workflow Guidelines

This document outlines guidelines for process management and development workflow within the project, aimed at ensuring a stable and predictable runtime environment.

## Child Process Management

Managing child processes correctly is essential to prevent system instability, such as resource leaks or unexpected termination.

-   On Unix-like systems, a `SIGCHLD` handler with `SA_NOCLDWAIT | SA_RESTART` is installed at startup. This automatically reaps any unexpected subprocesses, preventing "zombie" (`<defunct>`) children.
-   If new child processes are introduced, ensure platform-specific handling is compatible with this policy. On Unix, this means either:
    -   Collecting the exit status through a mechanism that does not rely on `waitpid`.
    -   Temporarily overriding the signal disposition before spawning the child process and restoring it afterward.
-   Non-Unix builds currently use a no-op stub for this handler. If platform-specific process management is introduced on other targets, equivalent safeguards must be implemented.

## Workflow Best Practices

Adhering to these practices ensures code quality and consistency:

-   **Centralize Shared Logic**: All shared functionality must be located in `crates/core`. The `crates/cli` must remain a thin wrapper, focusing solely on command-line interface concerns.
-   **Changelog Updates**: Update `docs/CHANGELOG.md` for all user-visible changes, including new CLI flags, modifications to structured output shapes, changes in `AGENTS` synchronization behavior, and adjustments to priority rules.
-   **Pre-Publishing Checks**: Before publishing any changes, always run `cargo fmt` to ensure code formatting adheres to standards and `cargo test` to verify test integrity.
