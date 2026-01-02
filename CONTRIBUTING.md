# Contributing to Skrills

Thanks for contributing. Focus on API stability, matching existing patterns, and adding tests for new behaviors.

## Pre-Commit Checklist
- Run `make precommit` (fmt, lint, md-lint, test) before submitting a PR.
- Build documentation for API changes: `cargo doc --workspace --all-features --no-deps`.
- Update [`docs/CHANGELOG.md`](docs/CHANGELOG.md) for user-visible changes.

### Git Hook Setup
Run [`./scripts/install-git-hooks.sh`](scripts/install-git-hooks.sh) once to automatically run `make precommit` on every `git commit`.

## Public API & Stability
Stable public APIs are critical.
- **Pre-1.0**: We maintain best-effort compatibility for the documented public API (specifically `run` and `runtime`). See [`docs/semver-policy.md`](docs/semver-policy.md).
- **Check Compatibility**: Prevent breaking changes by running `cargo +nightly public-api diff --deny removed --deny changed origin/master..HEAD` in `crates/server`. CI enforces this.
- **Evolution**: Follow [Rust RFC 1105](https://rust-lang.github.io/rfcs/1105-api-evolution.html). Prefer additive changes.

## Tests
- **Regression Tests**: Add tests for new behaviors, especially MCP tool outputs.
- **Hermeticity**: Isolate `HOME` and write only to temporary directories in integration tests.
- **Gateway Testing**: Use `SKRILLS_GATEWAY_METRICS_PATH=/metrics` or `SKRILLS_GATEWAY_READY_REQUIRE_AUTH=1` for gateway tests. Iterate with `cargo test -p gateway --lib http::`.

## Documentation
- Sync [`README.md`](README.md) and mdBook with new commands or flags.
- Document changes in [`docs/CHANGELOG.md`](docs/CHANGELOG.md).
