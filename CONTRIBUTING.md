# Contributing

Thanks for helping improve skrills! Please keep the public surface
stable, follow the existing patterns, and add tests for new behavior.

## Quick checklist
- Run `make fmt lint test --quiet` (or `cargo fmt && cargo clippy && cargo test --workspace --all-features`).
- Build docs when touching public APIs: `cargo doc --workspace --all-features --no-deps`.
- Update `docs/CHANGELOG.md` for user-visible changes.

## Public API & stability
- Pre-1.0, best-effort compatibility for the documented surface (`run` entrypoint and `runtime` module). See `docs/semver-policy.md` for details.
- Public API guard: from `crates/server`, run `cargo +nightly public-api diff --deny removed --deny changed origin/master..HEAD` before opening a PR. CI enforces the same check.
- Follow [RFC 1105](https://rust-lang.github.io/rfcs/1105-api-evolution.html) when evaluating potential breakage; prefer additive changes.

## Tests
- Add regression tests for new runtime behaviors, especially MCP tool outputs.
- Keep integration tests hermetic by isolating `HOME` and writing to temp dirs only.

## Documentation
- Keep README and the mdBook in sync for new commands, tools, or flags.
- Note any behavior or surface changes in `docs/CHANGELOG.md` and link to them where helpful.
