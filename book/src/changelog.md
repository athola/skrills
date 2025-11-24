# Changelog (highlights)

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
