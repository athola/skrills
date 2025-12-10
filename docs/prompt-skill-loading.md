# Prompt skill loading

You want to know two things: (1) how skills get into the prompt, and (2) how we
know it really happens. Here are the two paths.

## Codex CLI (MCP tool call from client)

**Mechanism**
- Codex discovers the `skrills` MCP server via the `available_agents` entry in
  `~/.codex/AGENTS.md` (created by `skrills sync` or installer) which points to
  `skrills serve`.
- On each user prompt, the Codex client calls the MCP tool
  `autoload-snippet` with the user message as the `prompt` argument.
- The tool response includes the rendered skills bundle in
  `hookSpecificOutput.additionalContext`. Codex then appends that string to the
  model prompt. This is an explicit MCP tool call, not a hidden hook.

**How skills are chosen**
- Discovery: `skill_roots` → `discover_skills`, respecting
  `SKRILLS_INCLUDE_CLAUDE`, `SKRILLS_INCLUDE_MARKETPLACE`, manifest overrides,
  `--skill-dir`, and cache (`~/.codex/skills-cache.json` or `SKRILLS_CACHE_PATH`)
  with TTL.
- Relevance: pins + history + AGENTS preload + prompt keywords + trigram
  similarity. The user prompt is the CLI argument/STDIN text passed to
  `skrills emit-autoload` (or `SKRILLS_PROMPT`). The threshold comes from the
  CLI flag or `SKRILLS_EMBED_THRESHOLD`.
- Invalidation: when the cache is cleared or empty, the server reloads the
  snapshot first; if a subsequent scan yields no skills it keeps the snapshot so
  snapshot-only skills stay available.
- Render: manifest (minimal or full) plus content (Dual mode for Codex), bounded
  by `max_bytes` (`SKRILLS_MAX_BYTES` or flag) with manifest-only/gzip fallback.

**How it runs on every prompt**
- Codex clients are instructed (see AGENTS template text) to call
  `autoload-snippet` first for each user message. The MCP server is already
  registered in `AGENTS.md`, so the call is just a tool invocation over stdio.

**Proof / quick check**
- Run `skrills emit-autoload --prompt "my task"` and inspect
  `additionalContext`; you should see `[skills] ...` plus manifest/content.
- Matched skill names are recorded in `skills-history.json` for auditability.

**Efficiency & safety**
- Cached discovery, size caps, preview truncation, and deterministic
  render-mode selection keep the payload small and predictable.

## Claude Code (host assembles the prompt)

**Mechanism**
- Claude Code does *not* run `EmitAutoload`. It calls our MCP server:
  `listResources` to list skills and `readResource` to fetch them.
- Client identity `claude-desktop` makes `manifest_render_mode` choose
  ManifestOnly; we return names/hashes/previews. Claude Code inserts them into
  the prompt on its side.

**Why this works**
- MCP `listResources/readResource` is the contract Claude Code consumes; serving
  the manifest satisfies what the client needs to build its prompt.

**Efficiency & safety**
- Same discovery and caching as Codex, but manifest-only keeps payloads small;
  no gzip/content fallback needed.

## Key switches (both paths)
- `SKRILLS_INCLUDE_CLAUDE`, `SKRILLS_INCLUDE_MARKETPLACE`
- `SKRILLS_MANIFEST` (priority override), `SKRILLS_CACHE_PATH` (cache location)
- `SKRILLS_EMBED_THRESHOLD`, `SKRILLS_MAX_BYTES`, `SKRILLS_MANIFEST_MINIMAL`
