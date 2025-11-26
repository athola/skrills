# Runtime options (autoload render)

- Stored at `~/.codex/skills-runtime.json` when set via MCP.
- Mutable at runtime via MCP tools (no restart required).

## Tools
- `runtime-status`
  - Input: none
  - Output: effective `manifest_first`, `render_mode_log`, overrides, env
- `set-runtime-options`
  - Input JSON: `{ "manifest_first": bool?, "render_mode_log": bool? }`
  - Persists overrides to `~/.codex/skills-runtime.json`
- `render-preview`
  - Input: same args as `autoload-snippet` (`prompt`, `embed_threshold`, `include_claude`, `max_bytes`, `auto_pin`, `diagnose`)
  - Output: matched skill names, manifest byte size, estimated token count, truncation flags; useful for logging/gating payloads before injection

### CLI hints
- Check the installed version: `codex-mcp-skills --version` (pre-1.0, best-effort compatibility).
- Discover the available commands: `codex-mcp-skills --help` (MCP tools include `runtime-status` and `set-runtime-options`).

## Precedence
runtime overrides > env (`CODEX_SKILLS_MANIFEST_FIRST`, `CODEX_SKILLS_RENDER_MODE_LOG`) > manifest file defaults.

## Notes
- Render mode log emits at INFO when enabled.
- The allowlist and handshake process determine manifest-only, dual, or content-only behavior. Overrides can safely enforce legacy or manifest-first behavior.
