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
`skrills` distinguishes itself with its MCP server, engineered in Rust for performance and memory safety. This server, combined with a Codex hook, cross-agent synchronization capabilities, and streamlined installers, enables skills to function as runtime resources. Many other skill efforts, by contrast, typically include:
-   Static skill bundles that require manual copying.
-   CI pipelines that render `SKILL`-like documentation into prompts during build time.
-   Shared rule repositories that lack automation or an MCP layer.
-   Local-only synchronization CLIs.
-   Tutorials or how-to guides that offer no automation.

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
