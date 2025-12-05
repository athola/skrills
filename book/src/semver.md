# Public API and SemVer Policy

This project is currently in its `0.x` release series. During this phase of rapid iteration, API evolution follows the principles in [Rust RFC 1105](https://rust-lang.github.io/rfcs/1105-api-evolution.html).

- **Documented Public API**: The officially documented public API surface primarily includes the `run` entrypoint and the `runtime` module (used by `runtime-status` and `set-runtime-options`). All other components are internal and may change without notice.
- **Best-Effort Compatibility**: While we try to avoid introducing breaking changes to the documented public API, the interface is still evolving. New fields may be added, or existing behaviors adjusted, as the API stabilizes. All such breaking changes, should they occur, will be documented in [`docs/CHANGELOG.md`](docs/CHANGELOG.md).
- **Feature Gates**: The `watch` feature, which provides filesystem watching, is optional. It can be disabled for minimal builds to reduce binary size.
- **Tooling Guardrails**: Our Continuous Integration (CI) pipeline includes a public-API check to detect accidental changes to the API surface. Contributors should run this check locally before submitting pull requests; refer to [`CONTRIBUTING.md`](CONTRIBUTING.md) for detailed instructions.

Until 1.0 is released, we strongly recommend pinning dependencies to an exact minor release. Always review the changelog for migration notes about releases that might affect the public API.