# codex-mcp-skills

Rust MCP server that exposes local `SKILL.md` files as MCP resources and tools. It mirrors Claude skills, auto-loads relevant skills into Codex prompts, and keeps AGENTS.md in sync for non-MCP agents.

## Contents
- [`crates/core`](crates/core): MCP server and library.
- [`crates/cli`](crates/cli): Binary wrapper (`codex-mcp-skills`).
- [`scripts/`](scripts): Install + sync helpers.
- [`docs/`](docs): Process guidance and roadmap (`process-guidelines.md`, `plans/2025-11-22-skill-autoload-mcp.md`).

## Features
- MCP server over stdio exposing every discovered `SKILL.md` as `skill://<source>/<relative>`.
- Autoload tool that filters by prompt terms, pins, and priority; emits structured JSON.
- Duplicate-aware priority ordering: codex → mirror → claude → agent.
- Universal sync: optional copy to `~/.agent/skills` for multi-agent reuse.
- TUI for pin/unpin and optional Claude→Codex sync.
- AGENTS.md sync: embeds `<available_skills>` XML with per-skill priority rank.
- Structured outputs with `_meta` (priority, duplicates, ranks).

## Installation

### Standalone installers

We were inspired by the maintainers at [uv](https://github.com/astral-sh/uv?tab=readme-ov-file#installation) to create an intuitive and extensible install script.

```bash
# macOS / Linux
curl -LsSf https://raw.githubusercontent.com/${CODEX_SKILLS_GH_REPO:-athola/codex-mcp-skills}/main/scripts/install.sh | sh

# Windows
powershell -ExecutionPolicy ByPass -c "irm https://raw.githubusercontent.com/athola/codex-mcp-skills/main/scripts/install.ps1 | iex"
```
Environment overrides:
- `CODEX_SKILLS_GH_REPO` (default `athola/codex-mcp-skills`)
- `CODEX_SKILLS_VERSION` (tag without `v`, default latest)
- `CODEX_SKILLS_BIN_DIR` (default `~/.codex/bin`)
- `CODEX_SKILLS_TARGET` to force a specific target triple.

Release asset naming:
- Archives must include the target triple in the filename, e.g. `codex-mcp-skills-x86_64-unknown-linux-gnu.tar.gz`.
- Archive root should contain the binary `codex-mcp-skills` (`.exe` on Windows).
- Default release repo: `athola/codex-mcp-skills`; override with `CODEX_SKILLS_GH_REPO` if using a fork.

### From source
```bash
cargo install --path crates/cli --force
```

### One-step hook + MCP registration (local checkout)
```bash
./scripts/install-codex-skills.sh [--universal] [--universal-only]
```
- Hook written to `~/.codex/hooks/codex/prompt.on_user_prompt_submit`
- MCP server registered in `~/.codex/mcp_servers.json`
- `--universal` also copies skills into `~/.agent/skills`; `--universal-only` performs just that copy.

## Quick start
```bash
codex-mcp-skills serve                  # start MCP server
codex-mcp-skills list                   # view discovered skills
codex-mcp-skills emit-autoload --prompt "python testing" --diagnose
codex-mcp-skills tui                    # interactive pin/sync
```
Trigger any Codex prompt; the hook injects filtered `additionalContext` automatically.

## Usage
### Commands (CLI)
- `serve [--skill-dir DIR]...`
- `emit-autoload [--include-claude] [--max-bytes N] [--prompt TEXT] [--auto-pin] [--diagnose] [--skill-dir DIR]...`
- `list`, `list-pinned`, `pin ...`, `unpin ...`, `autopin --enable/--disable`, `history [--limit N]`
- `sync` (mirror Claude → Codex)
- `sync-agents [--path AGENTS.md] [--skill-dir DIR]...` (writes `<available_skills>` XML with ranks)
- `tui` (optional sync, then checkbox pinning with source/location/priority)

### Structured outputs
`list-skills` returns:
```json
{
  "skills": [{ "name": "...", "source": "...", "priority_rank": 1 }],
  "skills_ranked": [...sorted by priority_rank...],
  "_meta": {
    "duplicates": [{ "name": "...", "kept_source": "codex", "skipped_source": "claude" }],
    "priority": ["codex","mirror","claude","agent"],
    "priority_rank_by_source": { "codex": 1, "mirror": 2, "claude": 3, "agent": 4 }
  }
}
```
`autoload-snippet` includes `content`, `matched`, `skills`, and the same `_meta`.
`readResource` responses carry `_meta.location` and `_meta.priority_rank`.

### Skill discovery & priority
1) `~/.codex/skills`  
2) `~/.codex/skills-mirror`  
3) `~/.claude/skills`  
4) `~/.agent/skills`  
Duplicates are skipped in lower-priority roots. Override with `~/.codex/skills-manifest.json`:
```json
{ "priority": ["agent","codex"], "expose_agents": false }
```
Missing roots auto-append in default order. Environment override: `CODEX_SKILLS_EXPOSE_AGENTS=true|false`.

### Universal skill mirror
```bash
./scripts/install-codex-skills.sh --universal-only
```
Copies `~/.codex/skills` + mirror into `~/.agent/skills` (non-destructive). Env knobs: `CODEX_SKILLS_BIN`, `AGENT_SKILLS_DIR`, `CODEX_SKILLS_DIR`, `CODEX_MIRROR_DIR`.

### AGENTS.md sync
```bash
codex-mcp-skills sync-agents --path AGENTS.md
```
Injects:
```xml
<!-- available_skills:start -->
<available_skills generated_at_utc="..." priority="codex,mirror,claude,agent">
  <skill name="alpha/SKILL.md" source="codex" location="global" path="/home/u/.codex/skills/alpha/SKILL.md" priority_rank="1" />
</available_skills>
<!-- available_skills:end -->
```

## Development
- Workspace: `crates/core` (lib) + `crates/cli` (bin).
- Format/tests: `cargo fmt && cargo test`.
- Helpful scripts: `scripts/install-codex-skills.sh`, `scripts/install-codex-skills.sh --universal-only`.
- Internal docs: [docs/process-guidelines.md](docs/process-guidelines.md), [docs/plans/2025-11-22-skill-autoload-mcp.md](docs/plans/2025-11-22-skill-autoload-mcp.md).

## Roadmap / future work
- Awaiting rmcp resource-level `_meta` to expose `location/priority` directly in `listResources`.
- Optional richer TUI actions (toggle include-claude/auto-pin).

## License
MIT License © 2025 athola. See [LICENSE](LICENSE).

## Acknowledgements
README structure inspired by high-signal Rust projects like Tokio, emphasizing overview → install → quick start → examples → support.
