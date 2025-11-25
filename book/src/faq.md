# Frequently Asked Questions (extended)

### Why did the installer URL with `/main/` 404?
The repository default branch is `master`. Use `/HEAD/` in the raw URL so it resolves regardless of the default branch:
```
curl -LsSf https://raw.githubusercontent.com/${CODEX_SKILLS_GH_REPO:-athola/codex-mcp-skills}/HEAD/scripts/install.sh | sh
```

### Which release asset maps to my machine?
Check your Rust/Cargo target triple (e.g., `rustc -vV | grep host`). Download the archive whose filename ends with that triple, such as `codex-mcp-skills-x86_64-apple-darwin.tar.gz`. Windows builds include `.exe` inside.

### Codex shows `MCP startup failed: missing field "type"`. How do I fix it?
Re-run the installer (`install.sh` or `install.ps1`). It now registers `codex-skills` with `type = "stdio"` in both `~/.codex/mcp_servers.json` and `~/.codex/config.toml`. Manual fix: add `type: "stdio"` to the `codex-skills` entry in `mcp_servers.json` and `type = "stdio"` under `[mcp_servers."codex-skills"]` in `config.toml`, then restart Codex. Run `codex-mcp-skills doctor` to confirm both files are present, readable, and pointing at the right binary.

If it still fails, Codex is rejecting a tool schema (missing `type`, `integer`, or unions). Our tools now include `type: "object"`, but you can harden Codex for third‑party servers by proxying them through a schema normalizer:

```toml
[mcp_servers.firecrawl]
type = "stdio"
command = "npx"
args = ["-y", "codex-mcp-wrapper", "--", "npx", "-y", "@mendable/firecrawl-mcp"]
```

Swap the final command for the server you want to wrap. The wrapper (github.com/kazuhideoki/codex-mcp-wrapper) injects missing `type`, converts `integer` → `number`, flattens unions, and filters schemas Codex rejects. To capture what Codex dislikes, run `codex-mcp-skills serve --trace-wire` and share the log.

### How does this project compare to other skill efforts?
`codex-mcp-skills` stands out primarily due to its MCP server, which is built in Rust for performance and memory safety. This server, along with a Codex hook, cross-agent synchronization capabilities, and simplified installers, allows skills to be served as runtime resources. In contrast, many other skill efforts often fall into categories such as:
-   Static skill bundles that require manual copying.
-   CI pipelines that render `SKILL`-like documentation into prompts during build time.
-   Shared rule repositories that lack automation or an MCP layer.
-   Local-only synchronization CLIs.
-   Tutorials or how-to guides that offer no automation.

For a more detailed comparison, please refer to the [Comparison to Similar Projects](./comparison.md) section.

### Can I keep Claude and Codex skills in sync automatically?
Yes. Use `codex-mcp-skills sync` to mirror Claude skills into Codex paths, and the autoload hook will surface them on prompt submission.

### Does the MCP server expose everything on disk?
No. It only reads configured directories (`--skill-dir` flags or defaults). Use separate paths for trusted vs. untrusted skills and avoid passing sensitive files.

### How do I contribute new skills?
Add them to your skills directory and rerun `codex-mcp-skills list` to verify discovery. For upstream contribution, follow the repo’s PR process and include tests or sample prompts when relevant.
