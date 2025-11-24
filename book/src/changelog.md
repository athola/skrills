# Changelog (highlights)

## 0.1.3
- Installer now builds from source in a temp cargo home when no release asset
  is available, then installs to the requested bin dir.
- Asset selection prefers `jq` with an awk fallback (no Python); pipefail is
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
  (opt-out via CODEX_SKILLS_NO_HOOK; universal sync via CODEX_SKILLS_UNIVERSAL).
- mdBook added with Pages deploy; Makefile gains book/docs targets and CLI demos.
- CI workflow runs fmt, clippy, tests, docs, and mdbook; release assets named
  `codex-mcp-skills-<target>.tar.gz`.
- Docs expanded on child-process safety, cache TTL manifest option, and
  installer defaults.

## 0.1.0
- Workspace split: `crates/core` (MCP server/lib) and `crates/cli` (binary).
- Structured `_meta` across tools with priority ranks and duplicate info.
- AGENTS.md sync includes per-skill priority rank and priority list.
- README refreshed: install/usage, universal sync, TUI, structured outputs.

For full details see `docs/CHANGELOG.md` in the repo.
