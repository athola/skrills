# Process Handling and Workflow Guidelines

This document outlines guidelines for process management and development workflow within the project, ensuring a robust and predictable runtime environment.

## Child Process Management

Proper management of child processes is crucial for system stability and to prevent resource leaks.

-   On Unix-like systems, a `SIGCHLD` handler with `SA_NOCLDWAIT | SA_RESTART` is installed at startup. This automatically reaps any unexpected subprocesses, preventing "zombie" (`<defunct>`) children.
-   If you introduce new child processes, ensure your platform-specific handling is compatible with this policy. On Unix, this means either:
    -   Collecting the exit status through a mechanism that does not rely on `waitpid`.
    -   Temporarily overriding the signal disposition before spawning the child process and restoring it afterward.
-   Non-Unix builds currently use a no-op stub for this handler. If you introduce platform-specific process management on other targets, you must implement equivalent safeguards.

## Workflow Best Practices

Adhering to these practices helps maintain code quality and consistency:

-   **Centralize Shared Logic**: All shared functionality should reside in `crates/core`. The `crates/cli` should remain a thin wrapper, focusing solely on command-line interface concerns.
-   **Changelog Updates**: Update `docs/CHANGELOG.md` for all user-visible changes. This includes new CLI flags, modifications to structured output shapes, changes in `AGENTS` synchronization behavior, and adjustments to priority rules.
-   **Pre-Publishing Checks**: Before publishing any changes, always run `cargo fmt` to ensure code formatting adheres to standards and `cargo test` to verify test integrity.
