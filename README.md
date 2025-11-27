# skrills
![Skrills icon](assets/icon.png)
Modern Rust MCP server + CLI for discovering, filtering, and injecting local `SKILL.md` files with minimal token overhead.

[![CI](https://img.shields.io/github/actions/workflow/status/athola/skrills/ci.yml?branch=master)](https://github.com/athola/skrills/actions/workflows/ci.yml)
[![Coverage](https://img.shields.io/github/actions/workflow/status/athola/skrills/coverage.yml?branch=master&label=coverage)](https://github.com/athola/skrills/actions/workflows/coverage.yml)
[![Codecov](https://codecov.io/gh/athola/skrills/branch/master/graph/badge.svg)](https://codecov.io/gh/athola/skrills)
[![Security Audit](https://img.shields.io/github/actions/workflow/status/athola/skrills/audit.yml?branch=master&label=audit)](https://github.com/athola/skrills/actions/workflows/audit.yml)
[![Docs](https://img.shields.io/github/actions/workflow/status/athola/skrills/book-pages.yml?branch=master&label=docs)](https://github.com/athola/skrills/actions/workflows/book-pages.yml)

---

## Install
```bash
cargo install skrills
```

From source:
```bash
# from repo root
cargo install --path crates/cli
```

## Publishing
- Releases are built from tags `v*` and binaries are attached to GitHub Releases.
- Crates are published in order (`skrills-state`, `skrills-discovery`, `skrills-server`, `skrills`) after a release is published.
- CI requires `CARGO_REGISTRY_TOKEN` in repo secrets for publishing; PRs run `cargo publish --dry-run` for all crates.

## Error handling & panics
- Library code returns `Result` for recoverable errors. If an insurmountable bug is encountered, then panic (e.g., unreachable states).
- CLI exposes errors with context. Panics should be treated as bugs and be reported.

## How prompt submission is wired
- Codex does not ship a built-in `UserPromptSubmit` hook. The installer adds a client-side hook file at `~/.codex/hooks/codex/prompt.on_user_prompt_submit`.
- That hook invokes the local `skrills` MCP server on every prompt submission. `skrills` runs `render_autoload` to tokenize/embed the prompt, filter skills, apply pins/byte limits, and return a manifest-first payload plus diagnostics.
- The client injects the MCP response into `additionalContext`, making matched `SKILL.md` content available for that prompt. If the hook is missing, skrills will not run—reinstall or create the hook manually pointing to `skrills serve`/autoload.

## Why This Project?

This project was created to solve a few specific problems:

- **Local Skill Management:** We wanted a way to manage `SKILL.md` files locally without losing the ability to discover and use them through a standardized MCP (Machine-checked Proof) contract.
- **Efficient Prompting:** Large, unfiltered prompts are expensive and slow. This tool filters skills based on prompt content and embeddings to keep prompts small and relevant.
- **Clear Diagnostics:** To understand why a skill was or wasn't included, the server provides detailed manifests and diagnostics. This makes integration and debugging more reliable for client tools.
- **Codex Issue:** It also addresses a [long-standing issue in the Codex repository](https://github.com/openai/codex/issues/5291) related to skill management.

## At a glance
- **Language:** Rust workspace (server/CLI, discovery, state crates)
- **Primary use:** Autoload skills at `UserPromptSubmit` with manifest-first output, with an option for legacy concatenation.
- **Clients:** Any MCP-capable tool (Codex CLI, Claude, custom agents).

### Prebuilt scripts
```bash
# macOS / Linux
curl -LsSf https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh | sh

# Windows
powershell -ExecutionPolicy Bypass -NoLogo -NoProfile -Command "Remove-Item alias:curl -ErrorAction SilentlyContinue; iwr https://raw.githubusercontent.com/athola/skrills/HEAD/scripts/install.ps1 -UseBasicParsing | iex"
```
Flags: `--local` builds from checkout; env `SKRILLS_BIN_DIR`, `SKRILLS_TARGET`, `SKRILLS_VERSION`, `SKRILLS_GH_REPO` override defaults.
Installers now also remove legacy `codex-mcp-skills` binaries and MCP entries to prevent duplicate server registrations.

### Supported platforms
- Linux, macOS, Windows (x86_64); other targets supported by cargo builds.
- MCP transport: stdio (required by newer Codex MCP clients).

## Quickstart
```bash
skrills serve                                   # start MCP server
skrills emit-autoload --prompt "rust tests"     # prompt-filtered manifest+content
skrills render-preview --prompt "rust tests"    # see matched skills + token estimate (no content)
skrills pin context-optimization                # keep a skill always included
```

## How autoload works
```mermaid
flowchart LR
  A["User prompt / submit hook"] --> B["Skill index (cached discovery)"]
  B --> C{Filter}
  C -- "keywords" --> D["Match set"]
  C -- "pins / auto-pin" --> D
  C -- "trigram similarity (embed_threshold)" --> D
  D --> E["Manifest builder\nmax_bytes + include_claude"]
  E --> F["Diagnostics + manifest-first payload"]
  F --> G["Client injects additionalContext"]
  D -.-> H["render-preview tool"] -.-> G
```

## Core features
- **Prompt + embedding filtering:** 
  * Converts prompt terms to tokens and applies a trigram-similarity algorithm (`--embed-threshold`, env `SKRILLS_EMBED_THRESHOLD`) to catch typos while preventing inclusion of non-relevant data
- **Manifest-first payloads:** 
  * `[skills] ...` summary with additional JSON manifest and full content
  * Toggle the legacy concat capability with `SKRILLS_MANIFEST_FIRST=false`
- **Diagnostics & observability:** 
  * Provides access to included/skipped/truncated lists of skills
  * Duplicate skills automatically resolved by priority
  * `render-preview` reports matched skills, byte size, and token estimate before injection into the prompt
- **Pinning:** 
  * Manually pin the skill to additional prompts plus heuristic auto-pin from recent history
- **Runtime overrides via MCP:** 
  * `runtime-status`, `set-runtime-options`
  * Persisted to `~/.codex/skills-runtime.json`.
- **Claude mirror support:** 
  * Optional sync from `~/.claude/skills` into `~/.codex/skills-mirror`.

## MCP tools
- `list-skills` — enumerate discovered skills with source and hash.
- `autoload-snippet` — prompt-filtered manifest + content (adheres to existing pinned skills and token budgets).
- `render-preview` — matched skill names, manifest bytes, estimated tokens, truncation flags (no content).
- `runtime-status` / `set-runtime-options` — inspect/persist render overrides (`manifest_first`, `render_mode_log`, `manifest_minimal`).
- `sync-from-claude`, `refresh-cache` — maintenance tasks to improve user experience.

## Configuration
- Env: `SKRILLS_MAX_BYTES`, `SKRILLS_MANIFEST_FIRST`, `SKRILLS_EMBED_THRESHOLD`, `SKRILLS_RENDER_MODE_LOG`, `SKRILLS_INCLUDE_CLAUDE`, `SKRILLS_CACHE_TTL_MS`.
- Manifest: `~/.codex/skills-manifest.json` for priority overrides, TTL, `expose_agents`, etc.
- CLI flags: `--skill-dir`, `--max-bytes`, `--prompt`, `--embed-threshold`, `--auto-pin`, `--include-claude`, `--diagnose`.

## Repository Structure
- `crates/server` — MCP server + CLI.
- `crates/discovery` — scanning, hashing, diagnostics.
- `crates/state` — pins/history persistence, env + manifest parsing.
- `crates/cli` — thin binary wrapper.
- `book/` — mdBook user and contributor docs (build via `make book`).
- `docs/` — changelog, runtime options, semver policy, process guidelines, plans.
- `scripts/` — install and sync helpers.

## Documentation links
- AI agent development: `AGENTS.md`
- User/deep-dive: `book/src/` (CLI, autoload, development, FAQ, Public API & SemVer).
- Runtime options & overrides: `docs/runtime-options.md`.
- SemVer + public API expectations: `docs/semver-policy.md`.
- Release artifacts/process: `docs/release-artifacts.md`, `docs/process-guidelines.md`.
- Changelog: `docs/CHANGELOG.md`.

## Status & roadmap
- Current: pre-1.0, manifest-first default, render-preview for observability, public-API guardrails (`cargo-public-api`).
- Near term: richer diagnostics (per-skill byte and token breakdown), Windows path defaults, optional HTTP transport.
- Tracking: see `docs/CHANGELOG.md` and `docs/plans/` for dated plans.

## Development
```bash
make fmt lint test --quiet
```
Public API guardrails: see `docs/semver-policy.md` and the mdBook “Public API & SemVer” page. CI also runs `cargo-public-api` for the server crate.

## Contributing & support
Issues and PRs welcome. Start with `skrills tui` to pin/sync locally, then follow `docs/process-guidelines.md`. For questions, file an issue with reproduction steps and `runtime-status` output.

## License
[MIT](./LICENSE)

## Star History

[![Star History Chart](https://api.star-history.com/svg?repos=athola/skrills&type=date&legend=top-left)](https://www.star-history.com/#athola/skrills&type=date&legend=top-left)
