# Contributing to Skrills

Thank you for considering contributing to `skrills`. To ensure a smooth and effective collaboration, we ask that you prioritize maintaining a stable public API, adhere to established coding patterns, and include comprehensive tests for all new behaviors.

## Pre-Commit Checklist
- Execute `make precommit` (which encompasses Rust formatting, linting, testing, and Markdown linting), or at a minimum `make fmt lint lint-md test --quiet`, before submitting any pull request.
- Build documentation when modifying public APIs using: `cargo doc --workspace --all-features --no-deps`.
- Update [`docs/CHANGELOG.md`](docs/CHANGELOG.md) for any user-visible changes.

### Git Hook Setup
- Execute [`./scripts/install-git-hooks.sh`](scripts/install-git-hooks.sh) once after cloning the repository. This action enables the repository-managed `pre-commit` hook, which automatically runs `make precommit` on every `git commit`.

## Public API & Stability
Maintaining a stable public API is a foundational principle of this project.
- Prior to version 1.0, we endeavor to maintain best-effort compatibility for the documented public API surface (specifically the `run` entrypoint and `runtime` module). Consult [`docs/semver-policy.md`](docs/semver-policy.md) for additional details.
- To proactively prevent inadvertent breaking changes, execute `cargo +nightly public-api diff --deny removed --deny changed origin/master..HEAD` from the `crates/server` directory before opening a pull request. This check is also rigorously enforced by the CI pipeline.
- Adhere to [Rust RFC 1105](https://rust-lang.github.io/rfcs/1105-api-evolution.html) when assessing potential API breakage, and favor additive changes over modifications.

## Tests
- Include regression tests for any new runtime behaviors, with particular emphasis on Machine-Readable Context Protocol (MCP) tool outputs.
- Maintain the hermetic nature of integration tests by isolating the `HOME` environment and directing all test-related writes exclusively to temporary directories.
- For testing gateway metrics and authentication, configure `SKRILLS_GATEWAY_METRICS_PATH=/metrics` or `SKRILLS_GATEWAY_READY_REQUIRE_AUTH=1`. You can iterate on these specific tests using `cargo test -p gateway --lib http::`.

## Documentation
- Ensure that the [`README.md`](README.md) and the mdBook documentation are synchronized with any new commands, tools, or flags introduced.
- Document all behavior or API changes in [`docs/CHANGELOG.md`](docs/CHANGELOG.md).
