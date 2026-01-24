# Public API and SemVer Policy

This project is in its `0.x` release series and follows the API evolution guidelines in [Rust RFC 1105](https://rust-lang.github.io/rfcs/1105-api-evolution.html).

## Policy

- **Compatibility**: We aim for compatibility with the documented public API. However, the interface is evolving, and breaking changes may occur. These will be recorded in [`docs/CHANGELOG.md`](docs/CHANGELOG.md).
- **Feature Flags**: The `watch` feature (filesystem watching) is optional. Consider this feature gate when embedding the library.
- **Tooling Guardrails**: CI checks prevent accidental public API changes. Run local checks before submitting pull requests (see [`CONTRIBUTING.md`](CONTRIBUTING.md)).

Pin dependencies to an exact `0.x.y` version and review release notes for changes.

---
**Reference**: [RFC 1105 – API evolution](https://rust-lang.github.io/rfcs/1105-api-evolution.html).
