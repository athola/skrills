# Public API and SemVer policy

The project is currently in its `0.x` series. It follows Rust RFC 1105’s guidance on API
evolution while the interface is incubating:

- **Best-effort compatibility**: The project aims to avoid breaking changes to the documented
  surface. However, it reserves the right to make fixes or add fields. Breaking changes
  will be called out explicitly in `docs/CHANGELOG.md`.
- **Feature flags**: optional `watch` enables filesystem watching; keep this
  feature gate in mind when embedding.
- **Tooling guardrails**: CI runs public API checks to prevent accidental surface
  changes. Contributors are encouraged to perform these checks locally (see `CONTRIBUTING.md`).

Goal: reach 1.0 once the surface settles; until then, pin to an exact minor
release if you depend on the library and review release notes for changes.

References: [RFC 1105 – API evolution](https://rust-lang.github.io/rfcs/1105-api-evolution.html).
