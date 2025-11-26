# Public API and stability

The project is currently in its `0.x` series. It follows the principles of Rust RFC 1105
for API evolution during rapid iteration:

- **Documented surface**: embedding entrypoint `run` and the `runtime` module
  (used by the MCP tools `runtime-status` / `set-runtime-options`). Everything
  else is internal and may change without notice.
- **Best-effort compatibility**: The project aims to avoid breaking changes to the documented
  surface. However, it may add fields or adjust behavior as it stabilizes. Any breaking
  change will be documented in `docs/CHANGELOG.md`.
- **Feature gates**: the `watch` feature adds filesystem watching; disable it if
  you need a minimal build.
- **Guardrails**: CI runs a public-API check to flag accidental surface changes;
  contributors should run it locally (see `CONTRIBUTING.md`).

Pin to an exact minor release until 1.0. Review the changelog for migration
notes on releases that touch the documented surface.
