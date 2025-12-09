# Frequently Asked Questions

### Why does the installer URL with `/main/` sometimes fail?
The installer URL using `/main/` can fail if the repository's default branch is not named `main`. To avoid this issue, use `/HEAD/` in the raw URL:
```bash
curl -LsSf https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh | sh
```

### How do I identify the correct release asset for my system?
To find the correct release asset, determine your system's Rust/Cargo target triple (e.g., by running `rustc -vV | grep host`). Then, download the archive whose filename ends with that specific triple, for instance, `skrills-x86_64-apple-darwin.tar.gz`. Windows builds will have an `.exe` executable inside the archive.

### How can I resolve the `MCP startup failed: missing field "type"` error in Codex?
Re-execute the installer (`install.sh` or `install.ps1`). The updated installer will automatically register `skrills` with `type = "stdio"` in both [`~/.codex/mcp_servers.json`](~/.codex/mcp_servers.json) and [`~/.codex/config.toml`](~/.codex/config.toml).

**Manual fix**: Manually add `type: "stdio"` to the `skrills` entry in `mcp_servers.json` and `type = "stdio"` under `[mcp_servers."skrills"]` in `config.toml`. Then, restart Codex. Run `skrills doctor` to confirm that both files are correctly configured and accessible.

If the error persists, particularly with a third-party MCP server, it could be due to a schema rejection by Codex. In such cases, consider proxying problematic servers through a schema normalizer like `codex-mcp-wrapper`. An example configuration is provided below:
```toml
[mcp_servers.firecrawl]
type = "stdio"
command = "npx"
args = ["-y", "codex-mcp-wrapper", "--", "npx", "-y", "@mendable/firecrawl-mcp"]
```
Replace the final command in the example with the actual server you intend to wrap. The `codex-mcp-wrapper` (available at [github.com/kazuhideoki/codex-mcp-wrapper](https://github.com/kazuhideoki/codex-mcp-wrapper)) injects missing `type` fields, converts `integer` to `number` types, flattens unions, and filters out schemas Codex might reject. To capture schema rejections from Codex, run `skrills serve --trace-wire` and provide the log.

### How does `skrills` compare to other skill management initiatives?
`skrills` stands out with an MCP server implemented in Rust, which supports both Codex and Claude environments, and can sync skills between them. It also provides pre-built binaries and installers for easier deployment. In contrast, other projects often have static skill collections, CI-based documentation tools, or local synchronization utilities that lack an MCP layer. For a detailed comparative analysis, please refer to the [Project Comparison](./comparison.md) documentation.

### Is it possible to automatically synchronize skills between Claude and Codex?
Yes. Use `skrills mirror` (or `sync`, `sync-all`) to copy Claude skills/agents/commands into Codex paths. Add `--skip-existing-commands` to keep local command files; command sync is byte-for-byte, so non-UTF-8 commands are preserved. The installer runs a mirror automatically on Codex unless you set `SKRILLS_NO_MIRROR=1` or `~/.claude` is absent, in which case it prints a reminder to run `skrills mirror` later. The autoload hook then makes the mirrored skills available on submit.

### Can the autoload rendering behavior be modified at runtime?
Yes. The autoload rendering behavior can be adjusted at runtime with MCP tools:
- `runtime-status` lets you inspect the effective `manifest_first` and `render_mode_log` settings.
- `set-runtime-options` takes JSON parameters (e.g., `{ "manifest_first": false, "render_mode_log": true }`) to update these settings.
These overrides are stored in [`~/.codex/skills-runtime.json`](~/.codex/skills-runtime.json) and take precedence over both environment variables and manifest defaults.

### Does the MCP server expose all content from disk?
No. The MCP server only reads from configured directories, set either via `--skill-dir` flags or through default settings. Use separate paths for trusted and untrusted skills, and do not place sensitive files within these accessible directories.

### What is the process for contributing new skills?
To contribute new skills, first add them to your designated skills directory. Then, run `skrills list` to confirm discovery. For upstream contributions to the project, follow the repository’s PR process, including relevant tests or illustrative sample prompts.
