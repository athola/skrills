# Development Process and Workflow Guidelines

This document outlines development processes and safety considerations to maintain a stable and reliable runtime environment.

## Child Process Management

Effective child process management is critical to prevent system instability, like resource leaks or unexpected process termination.

- The process installs no `SIGCHLD` handler. Wait on every child you spawn: `Command::status()` and `Command::output()` do this for you, and a bare `spawn()` needs an explicit `wait()` or the child lingers as a zombie until skrills exits.
- Never set `SIGCHLD` to `SIG_IGN` or install a handler with `SA_NOCLDWAIT`. The kernel then reaps children itself, `waitpid` returns `ECHILD`, and every `Command::status()` or `Command::output()` in the process fails with "No child processes". An earlier startup handler did exactly this, and `analyze-project-context` returned an empty `git_keywords` list because its `git log` could not be waited on.
- Two checks guard the invariant through the real binary, since unit tests call command functions directly and never pass through `run()`: `crates/cli/tests/subprocess_wait.rs` under `cargo test`, and the "subcommands can still run subprocesses" scenario in `scripts/dogfood-contracts.sh`.

## Workflow Best Practices

Follow these practices to maintain code quality and development standards:

- **Keep the Crate Boundary**: `crates/cli` owns the command-line surface: the clap definitions, subcommand handlers, dispatcher, sync TUI, doctor and cold-window CLI. `crates/server` owns the MCP server and the engine those handlers call, and the leaf crates own their domains. Code both the MCP path and a subcommand need stays in `crates/server`, and `setup.rs` is there for that reason.
- **Changelog Updates**: Ensure the changelog is updated for all user-visible changes, including new CLI flags, changes to output formats, `AGENTS` sync behavior, and priority rule adjustments.
- **Pre-Publishing Checks**: Before publishing, run `cargo fmt` to ensure code formatting and `cargo test` to confirm test integrity.

## Development Checklist

Before finalizing and merging changes, consider the following checklist:

- Verify that no new zombie or unmanaged child processes are introduced.
- Confirm a changelog entry exists for all user-facing changes.
- Ensure all tests pass and code formatting adheres to standards.
- Confirm all new CLI flags are documented in both [`README.md`](README.md) and the project book.