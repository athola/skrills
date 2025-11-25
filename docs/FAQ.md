# Frequently Asked Questions

### Why did the installer URL with `/main/` fail?
Use the branch-agnostic path in the installer URL. For example: `https://raw.githubusercontent.com/${CODEX_SKILLS_GH_REPO:-athola/codex-mcp-skills}/HEAD/scripts/install.sh` (or `install.ps1` for Windows). This approach ensures the URL resolves correctly regardless of the repository's default branch.

### Which release asset should be downloaded manually?
Select the archive whose filename includes your specific target triple. For instance, a Linux system might use `codex-mcp-skills-x86_64-unknown-linux-gnu.tar.gz`. Each archive contains the corresponding binary at its root.

### Codex shows `MCP startup failed: missing field "type"`. How can this be resolved?
Reinstall using the latest installer (`install.sh` or `install.ps1`). The updated installer now registers `codex-skills` with `type = "stdio"` in both `~/.codex/mcp_servers.json` and `~/.codex/config.toml`.

For manual correction:
1.  Add `type: "stdio"` to the `codex-skills` entry within `mcp_servers.json`.
2.  Add `type = "stdio"` under `[mcp_servers."codex-skills"]` in `config.toml`.
3.  Restart Codex.

To verify the setup, run `codex-mcp-skills doctor` to confirm both files are present, readable, and correctly configured.

If the error persists, Codex might be rejecting a tool schema (often due to a missing top-level `type`). While our tools now include `type: "object"`, some third-party MCP servers may emit schemas that Codex rejects (e.g., using `integer` instead of `number`, or complex unions). You can address this by proxying such servers through a schema normalizer:

**Requirements**: Node.js 18+

**Configuration**: Add a wrapped entry that runs the normalizer in front of the real server:
```toml
[mcp_servers.firecrawl]
type = "stdio"
command = "npx"
args = ["-y", "codex-mcp-wrapper", "--", "npx", "-y", "@mendable/firecrawl-mcp"]
```
Replace the `command` and `args` with those of the problematic MCP server. The `codex-mcp-wrapper` (available on GitHub at `kazuhideoki/codex-mcp-wrapper`) injects missing `type` fields, converts `integer` to `number`, flattens unions, and filters unsupported schemas before they reach Codex.

To diagnose what Codex is rejecting, run `codex-mcp-skills serve --trace-wire` and examine the generated log.

### Does this project replace existing Claude skills directories?
No. The MCP server reads skills from default locations and can mirror Claude skills. It does not overwrite existing files unless explicit synchronization commands are executed.

### How does this project differ from other skill management efforts?
`codex-mcp-skills` provides an MCP server, a Codex hook, and cross-agent synchronization capabilities, transforming skills into runtime resources rather than just static files. Other efforts typically involve static skill bundles, CI-driven documentation rendering, or local-only synchronization tools. For a more detailed comparison, refer to the [Comparison to Similar Projects](../../book/src/comparison.md) section in the project book.

### How are the project's documentation built locally?
Use `make book` to build and open the `mdBook` documentation. For live-reloading during development, run `make book-serve` (accessible at `http://localhost:3000`). For Rust API documentation, use `make docs`.

### Does the system function offline?
Yes. Once the binary and necessary skills are present locally, the MCP server and CLI can operate without an active network connection.

### What are the security considerations?
The server operates over standard I/O (stdio) and performs file reads with the principle of least privilege. No secrets are bundled with the software, and you retain full control over which skill directories are exposed. Always review third-party skills before integrating them.

For more in-depth answers and advanced scenarios, consult the main project book's FAQ section.
