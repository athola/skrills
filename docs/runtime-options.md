# Runtime Options (Autoload Render)

Runtime options for autoload rendering are stored in [`~/.codex/skills-runtime.json`](~/.codex/skills-runtime.json) when configured through the Machine-Readable Context Protocol (MCP). These settings are dynamically modifiable at runtime using MCP tools, without requiring a service restart.

## Tools

- **`runtime-status`**: The `runtime-status` tool shows the current settings for `manifest_first` and `render_mode_log`, explicitly detailing any active overrides or influencing environment variables.
- **`set-runtime-options`**: The `set-runtime-options` tool saves runtime overrides to [`~/.codex/skills-runtime.json`](~/.codex/skills-runtime.json). It accepts a JSON input structure, such as `{ "manifest_first": bool?, "render_mode_log": bool? }`, to modify these settings.
- **`render-preview`**: The `render-preview` tool accepts the same arguments as `autoload-snippet`. Its output includes the names of matched skills, the manifest's byte size, an estimated token count, and any relevant truncation flags. This tool is useful for logging or for checking payloads before sending them to the LLM.

### CLI Usage
- To check the installed version of `skrills`, execute: `skrills --version`.
- To see all available commands and their functionalities, use: `skrills --help`. Note that MCP tools such as `runtime-status` and `set-runtime-options` are also included.

## Precedence

Runtime overrides have the highest precedence, overriding environment variables (e.g., `SKRILLS_MANIFEST_FIRST`, `SKRILLS_RENDER_MODE_LOG`). Environment variables, in turn, override any default settings defined within the manifest file itself.

## Notes

- When enabled, `render_mode_log` logs messages at the `INFO` logging level, providing details on rendering operations.
- The allowlist and handshake process determine whether the system runs in a `manifest-only`, `dual`, or `content-only` mode. Runtime overrides can be used to specifically enforce either legacy or manifest-first behaviors.
