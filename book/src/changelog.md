# Changelog (highlights)

## Unreleased
- Modularized workspace into `discovery`, `state`, `server`, and `cli`.
- Added coverage workflow (cargo-llvm-cov + Codecov) and badges.
- Added smoke tests for discovery hashing and state persistence/auto-pin.

## 0.1.14
- Added `doctor` diagnostics, `--trace-wire` logging, and schema hardening (`type = "object"`), plus installers that enforce `type = "stdio"` in Codex configs and support `--local` builds.

## 0.1.13
- Installer filters archives more strictly and uses source builds as a secondary option; CI jobs now gate on relevant path changes.

## 0.1.12
- Hardened release asset lookup in installers.

## 0.1.11
- Release workflow skips uploads when a release already exists.

## 0.1.10
- Create the GitHub release before uploading assets to avoid races.

## 0.1.9
- Fixed release upload include patterns so platform archives attach correctly.

## 0.1.8
- Respected manifest flag when hiding agents doc; cached audit workflow; corrected release include paths.

## 0.1.7
- Added release dry-run builds and cache reuse in CI.

## 0.1.6
- Switched to supported archive options in the release workflow for cross-platform packaging.

## 0.1.5
- Set the ZIP flag in the Windows upload step to produce valid artifacts.

## 0.1.4
- Fixed CI upload inputs for Rust binaries and stabilized env-driven tests.

## 0.1.3
- Installer now builds from source in a temp cargo home when no release asset
  is available, then installs to the requested bin dir.
- Asset selection prefers `jq` with an awk secondary (no Python); pipefail is
  optional for portability.
- Release assets use the action `{{ target }}` placeholder to publish the
  correct platform tarballs.

## 0.1.2
- Added social/brand icon (8-bit 1280×640) at `assets/icon.png` and linked it in README.
- Installer docs now use branch-agnostic `/HEAD/` URLs; README/book/FAQ updated.
- Makefile `book` target builds and opens the mdBook (no separate book-open target).
- Added comparison + FAQ chapters to the book and FAQ to `docs/`.
- Release workflow assets now interpolate target correctly; audit workflow runs `cargo audit` directly.

## 0.1.1
- One-command installer now wires the Codex hook/MCP registration by default
  (opt-out via SKRILLS_NO_HOOK; universal sync via SKRILLS_UNIVERSAL).
- mdBook added with Pages deploy; Makefile gains book/docs targets and CLI demos.
- CI workflow runs fmt, clippy, tests, docs, and mdbook; release assets named
  `skrills-<target>.tar.gz`.
- Docs expanded on child-process safety, cache TTL manifest option, and
  installer defaults.

## 0.1.0
- Workspace split: `crates/core` (MCP server/lib) and `crates/cli` (binary).
- Structured `_meta` across tools with priority ranks and duplicate info.
- AGENTS.md sync includes per-skill priority rank and priority list.
- README refreshed: install/usage, universal sync, TUI, structured outputs.

For full details see `docs/CHANGELOG.md` in the repo.
