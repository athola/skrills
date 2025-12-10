# How skills reach your prompt

This chapter walks through how skills reach prompts for Codex CLI (we execute a
pre-prompt command) and Claude Code (the host assembles the prompt). It follows
the path from filesystem roots to what each client receives.

## Codex CLI (MCP tool call)

### 1. Discovery and caching

- Roots come from priority order (`~/.codex`, mirror, Claude/cache/marketplace,
  agents) plus `--skill-dir` overrides. Environment switches:
  `SKRILLS_INCLUDE_CLAUDE`, `SKRILLS_INCLUDE_MARKETPLACE`, and a manifest
  override file (`SKRILLS_MANIFEST` or `~/.codex/skills-manifest.json`).
- A snapshot cache (`~/.codex/skills-cache.json`, override with
  `SKRILLS_CACHE_PATH`) plus TTL prevents full rescans. On invalidation or first
  access, the server reloads the snapshot before scanning; if the scan finds no
  skills, it keeps the snapshot so snapshot-only skills remain available.

### 2. Pins, history, preload terms

- Manual pins are read from `skills-pinned.json`; recent matches in
  `skills-history.json` can auto-pin.
- If `AGENTS.md` exists, its references seed preload keywords so agent docs can
  pull in related skills even before the user types a prompt.

### 3. Prompt-aware filtering

- The user prompt passed as the `prompt` argument to the `autoload-snippet` MCP
  tool (or `SKRILLS_PROMPT` if set on the server) is what we score skills
  against.
- `render_autoload` checks each skill:
  - Always keep pinned.
  - Keyword hit in name/preview keeps it.
  - Otherwise, compute trigram similarity between prompt and the skill preview
    (trimmed to `DEFAULT_EMBED_PREVIEW_BYTES`). Keep when it meets the embedding
    threshold (CLI flag or `SKRILLS_EMBED_THRESHOLD`, default 0.3).

### 4. Rendering modes

- The client implementation name decides render mode:
  - **Codex** → Dual (manifest + concatenated content).
  - **Claude desktop** → ManifestOnly.
  - Legacy clients may get ContentOnly.
- Minimal manifests drop paths/previews if `SKRILLS_MANIFEST_MINIMAL=1`.

### 5. Size governance and proof

- `max_bytes` (flag or `SKRILLS_MAX_BYTES`) bounds the output.
- If over limit, fall back to manifest-only, then gzipped/base64 manifest.
- Diagnostics (opt-in) record included/omitted skills and truncation.
- The `autoload-snippet` tool returns JSON with
  `hookSpecificOutput.additionalContext` containing the rendered bundle. Codex
  appends that string to the model prompt immediately after it receives the tool
  result.
- Matched skills are appended to history; caches persist for the next run.
- How it runs every time: Codex clients call the `autoload-snippet` MCP tool
  before responding to each user message (as instructed in `AGENTS.md`).

### Tips

- Testing in parallel? Point `SKRILLS_CACHE_PATH` at a temp file.
- Want a minimal manifest everywhere? Set `SKRILLS_MANIFEST_MINIMAL=1`.
- Exclude marketplaces by default: leave `SKRILLS_INCLUDE_MARKETPLACE` false or
  omit the corresponding CLI flag.

## Claude Code (host assembles the prompt)

- Claude Code desktop does not run our `EmitAutoload` command. It fetches skills
  via MCP (`listResources`/`readResource`) and builds the prompt itself.
- Client identity `claude-desktop` makes `manifest_render_mode` choose
  **ManifestOnly**; we return manifest data (names, hashes, previews) only.
- Discovery, pins, history, and marketplace flags match Codex; only the final
  prompt construction is on the Claude side.
- Size handling is simpler: manifest (or minimal manifest) must fit limits; no
  content/gzip fallback is used.

## Quick verification steps
- Codex: run `skrills emit-autoload --prompt "hello"` (same logic as
  `autoload-snippet`) and inspect `additionalContext` in the printed JSON (you
  should see `[skills] ...` plus manifest/content when under size limits).
- Claude Code: call MCP `listResources` and `readResource` and confirm the
  manifest entries; prompt assembly happens inside the Claude client.
