# Public API and SemVer Policy

This project is in its `0.x` release series. During this incubation period, we follow the API evolution guidelines in [Rust RFC 1105](https://rust-lang.github.io/rfcs/1105-api-evolution.html).

## Policy

- **Compatibility**: While we try to maintain compatibility with the documented public API, the interface is still evolving. Consequently, fixes or additions that might be breaking changes may be introduced. All such modifications are recorded in [`docs/CHANGELOG.md`](docs/CHANGELOG.md).
- **Feature Flags**: The `watch` feature, which enables filesystem watching, is optional. This feature gate should be considered when embedding the library into other projects to manage dependencies.
- **Tooling Guardrails**: Our Continuous Integration (CI) pipeline has checks to prevent accidental changes to the public API. You must run local checks before submitting pull requests; refer to [`CONTRIBUTING.md`](CONTRIBUTING.md) for detailed instructions.

Our goal is to achieve a stable 1.0 release once the public API is stable. Until then, we recommend that you pin your dependencies to an exact `0.x.y` version and carefully review the release notes for each update for any potential changes.

---
**Reference**: For further details on API evolution, please consult [RFC 1105 – API evolution](https://rust-lang.github.io/rfcs/1105-api-evolution.html).
