# Frequently Asked Questions (extended)

### Why did the installer URL with `/main/` 404?
The repository default branch is `master`. Use `/HEAD/` in the raw URL so it resolves regardless of the default branch:
```
curl -LsSf https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh | sh
```

### Which release asset maps to my machine?
Check your Rust/Cargo target triple (e.g., `rustc -vV | grep host`). Download the archive whose filename ends with that triple, such as `skrills-x86_64-apple-darwin.tar.gz`. Windows builds include `.exe` inside.

### Codex shows `MCP startup failed: missing field "type"`. How do I fix it?
Re-run the installer (`install.sh` or `install.ps1`). It now registers `skrills` with `type = "stdio"` in both `~/.codex/mcp_servers.json` and `~/.codex/config.toml`. Manual fix: add `type: "stdio"` to the `skrills` entry in `mcp_servers.json` and `type = "stdio"` under `[mcp_servers."skrills"]` in `config.toml`, then restart Codex. Run `skrills doctor` to confirm both files are present, readable, and correctly configured.
...
Our tools now include `type: "object"`, but you can enhance schema compatibility for third‑party servers by proxying them through a schema normalizer:

```toml
[mcp_servers.firecrawl]
type = "stdio"
command = "npx"
args = ["-y", "codex-mcp-wrapper", "--", "npx", "-y", "@mendable/firecrawl-mcp"]
```

Substitute the final command for the server you want to wrap. The wrapper (github.com/kazuhideoki/codex-mcp-wrapper) injects missing `type`, converts `integer` to `number`, flattens unions, and filters schemas that Codex rejects. To capture schema rejections from Codex, run `skrills serve --trace-wire` and share the log.

### How does this project compare to other skill efforts?
`skrills` is different from other skill management projects in a few ways:

-   **MCP Server**: It provides an MCP server written in Rust. This allows it to be used by both Codex and Claude.
-   **Synchronization**: It can synchronize skills between Codex and Claude.
-   **Installers**: It provides installers for pre-built binaries, so you don't have to build it from source.

Other skill management projects are often one of the following:
-   Static collections of skills that need to be manually copied.
-   CI pipelines that render documentation into prompts at build time.
-   Shared rule repositories without automation.
-   Local-only synchronization CLIs.
-   Tutorials or how-to guides without automation.

For a more detailed comparison, refer to the [Comparison to Similar Projects](./comparison.md) section.

### Can I keep Claude and Codex skills in sync automatically?
Yes. Use `skrills sync` to mirror Claude skills into Codex paths, and the autoload hook will surface them on prompt submission.

### Can I change autoload render behavior at runtime?
- Yes. Use MCP tools:
  - `runtime-status` to inspect effective `manifest_first` and `render_mode_log`.
  - `set-runtime-options` with JSON params, e.g. `{ "manifest_first": false, "render_mode_log": true }`.
- Overrides persist to `~/.codex/skills-runtime.json` and take precedence over env/manifest defaults.

### Does the MCP server expose everything on disk?
No. It only reads configured directories (`--skill-dir` flags or defaults). Use separate paths for trusted vs. untrusted skills and avoid passing sensitive files.

### How do I contribute new skills?
Add them to your skills directory and rerun `skrills list` to verify discovery. For upstream contribution, follow the repo’s PR process and include tests or sample prompts when relevant.
