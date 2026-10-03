# Contributing to Skrills

Thanks for contributing. Focus on API stability, matching existing patterns, and adding tests for new behaviors.

## Pre-Commit Checklist
- Run `make precommit` before submitting a PR. It runs `fmt-check` (checks formatting without rewriting; run `make fmt` to fix), `lint`, `lint-md`, `lint-hygiene`, `test`, `test-scripts`, `test-install`, `dogfood-precommit` and `verify-publish`.
- Build documentation for API changes: `cargo doc --workspace --all-features --no-deps`.
- Update [`docs/CHANGELOG.md`](docs/CHANGELOG.md) for user-visible changes.

### Git Hook Setup
Run [`./scripts/install-git-hooks.sh`](scripts/install-git-hooks.sh) once to automatically run `make precommit` on every `git commit`.

## Public API & Stability
We prioritize API stability to ensure reliable automation.
- **Pre-1.0**: We maintain best-effort compatibility for the documented public API (specifically `run` and `runtime`). See [`docs/semver-policy.md`](docs/semver-policy.md).
- **Check Compatibility**: Check for breaking changes by running `cargo +nightly public-api diff --deny removed --deny changed origin/master..HEAD` in `crates/server`. CI runs the same check and reports a diff as a warning on the run; it does not fail the build, so check the output yourself.
- **Evolution**: Follow [Rust RFC 1105](https://rust-lang.github.io/rfcs/1105-api-evolution.html). Prefer additive changes.

## Tests
- **Regression Tests**: Add tests for new behaviors, especially MCP tool outputs.
- **Hermeticity**: Tests must be hermetic. Isolate `HOME` and write only to temporary directories in integration tests.
- **HTTP and Gateway Testing**: The HTTP transport and the MCP gateway live in `skrills-server`. Iterate with `cargo test -p skrills-server --lib http_transport::` and `cargo test -p skrills-server --lib mcp_gateway::`, and run the end-to-end transport tests with `cargo test -p skrills-server --test http_transport_integration`. `skrills serve` takes its token, host allow-list and CORS origins from `SKRILLS_AUTH_TOKEN`, `SKRILLS_ALLOWED_HOSTS` and `SKRILLS_CORS_ORIGINS` (or the matching `~/.skrills/config.toml` keys). Tests that need them set them through `skrills_test_utils::set_env_var`, which restores the old value, rather than relying on your shell.

## Documentation
- Sync [`README.md`](README.md) and mdBook with new commands or flags.
- Document changes in [`docs/CHANGELOG.md`](docs/CHANGELOG.md).
