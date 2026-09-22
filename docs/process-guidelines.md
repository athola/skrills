# Process and Workflow Guidelines

## Child Process Management

The process installs no `SIGCHLD` handler, so wait on every child you spawn. `Command::status()` and `Command::output()` wait for you. A bare `spawn()` needs an explicit `wait()` or the child lingers as a zombie.

Never set `SIGCHLD` to `SIG_IGN` or use `SA_NOCLDWAIT`. The kernel then reaps children itself, `waitpid` returns `ECHILD`, and every `Command::status()` or `Command::output()` in the process fails with "No child processes". An earlier startup handler did this and broke each subcommand that shells out. `crates/cli/tests/subprocess_wait.rs` and the subprocess scenario in `scripts/dogfood-contracts.sh` guard it through the real binary.

## Workflow Best Practices

`crates/cli` owns the command-line surface (clap definitions, subcommand handlers, dispatcher, TUI, doctor), while `crates/server` owns the MCP server and the engine those handlers call. Code that both the MCP path and a subcommand need stays in `crates/server`, which is why `setup.rs` is there. Record user-visible changes such as new flags or output formats in `docs/CHANGELOG.md`. Always run `cargo fmt` and `cargo test` before publishing to maintain code quality.

## MCP Dependency Strategy

We define `rmcp` as a workspace dependency in the root `Cargo.toml`, enabling only required features in individual crates. Note that `pastey` is a transitive dependency used by `rmcp` for procedural macros and should not be added directly. When updating, bump the workspace version of `rmcp` and run all MCP integration tests, verifying `pastey` matches the version. Treat `rmcp` as supply-chain critical and monitor it for advisories.
