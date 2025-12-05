# Runtime Configuration and Tuning

Runtime overrides, saved in `~/.codex/skills-runtime.json`, allow on-the-fly adjustments to autoload rendering parameters through MCP tools, without service restarts. These adjustments are useful for debugging manifest parsing or controlling payload sizes.

## Tools

- **`runtime-status`**: This tool displays current overrides and relevant environment settings.
- **`set-runtime-options`**: This tool saves `manifest_first` and `render_mode_log` settings directly to the runtime configuration file.
- **`render-preview`**: Executes a dry-run of the autoload rendering process, providing size estimates before actual context injection.

**CLI Reminders**:
- **Check Version**: To check the installed version, run `skrills --version`.
- **Discover Commands**: To explore commands and their usage, run `skrills --help`.

## Precedence

Runtime overrides have the highest precedence and override environment variables such as `SKRILLS_MANIFEST_FIRST` and `SKRILLS_RENDER_MODE_LOG`. These environment variables, in turn, override any default settings in the manifest file. This hierarchy allows safe experimentation and dynamic adjustments without directly modifying skill files.

## Practical Recipes

- **Force Manifest-First**: To prioritize manifest-based skill loading, especially when client behavior is verbose, use `set-runtime-options '{"manifest_first": true}'`.
- **Enable Render Logging**: To debug truncation issues, enable render logging with `set-runtime-options '{"render_mode_log": true}'` and monitor logs for warnings related to payload size.
- **Gate Payloads**: Run `render-preview` with `max_bytes` and `embed_threshold` configured. This allows the system to block or warn if truncation flags are in the preview.