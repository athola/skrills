# Frequently Asked Questions

### Why did the installer URL with `/main/` fail?
The installer URL with `/main/` can fail if the repository's default branch is not named `main`. To avoid this issue, we use the branch-agnostic `/HEAD/` path in the installer URL: `https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh`. This special path automatically resolves to the latest commit on the default branch, ensuring the URL always works.

### Which release asset should I download manually?
When downloading a release manually, you should select the archive that matches your system's architecture, also known as the 'target triple'. For example, if you are on a 64-bit Linux system, you would download `skrills-x86_64-unknown-linux-gnu.tar.gz`. After extracting the archive, you will find the `skrills` binary at the root.

### How do I resolve the `MCP startup failed: missing field "type"` error in Codex?
This error indicates that the MCP server registration in your Codex configuration is missing the `type = "stdio"` field. To fix this, we recommend reinstalling with the latest installer (`install.sh` or `install.ps1`), which will handle the configuration for you.

If you prefer to fix it manually, you will need to:
1. Add `type: "stdio"` to the `skrills` entry in `~/.codex/mcp_servers.json`.
2. Add `type = "stdio"` under `[mcp_servers."skrills"]` in `~/.codex/config.toml`.
3. Restart Codex.

You can run `skrills doctor` to verify that both files are correctly configured.

If the error persists, it's possible that another MCP server is emitting a schema that Codex rejects. To diagnose this, you can run `skrills serve --trace-wire` and examine the log for schema validation errors. A potential workaround is to proxy the problematic server through a schema normalizer like `codex-mcp-wrapper`.

### Does this project replace my existing Claude skills?
No, this project is designed to be non-destructive. The MCP server reads skills from their default locations and can be configured to mirror your existing Claude skills. However, it will not overwrite any of your files unless you explicitly run a synchronization command (`skrills sync`). This allows you to try out `skrills` without affecting your current setup.

### How is this project different from other skill management tools?
The primary difference is that `skrills` treats skills as dynamic resources that can be managed and synchronized at runtime across different agents. This is achieved through its MCP server architecture. In contrast, other tools often rely on static skill bundles or provide only local synchronization. For a more detailed breakdown, please see our [Comparison to Similar Projects](../../book/src/comparison.md).

### How does autoloading remain token-efficient?
The autoloading feature is designed for efficiency. Instead of preloading all skills or accessing the disk on every interaction, it first parses the intent of your prompt. Then, it injects only the relevant skills from a pre-cached index. This process minimizes both latency and token consumption, while still respecting any pinned skills or byte-budget constraints you have set.

### How do I build the documentation locally?
You can build the documentation locally using our `Makefile`. To build and open the mdBook, run `make book`. If you want to have live-reloading as you make changes, use `make book-serve`. To generate the Rust API documentation, run `make docs`.

### Does the system work offline?
Yes, the system is designed to work offline. As long as you have the `skrills` binary and your skills stored on your local machine, both the MCP server and the command-line interface will function without an internet connection.

### What are the security considerations?
We've designed `skrills` with security in mind. The server communicates over standard I/O and operates with the least possible file access privileges. We do not bundle any secrets, and you have full control over which skill directories are exposed. As a general best practice, we always recommend that you review any third-party skills before using them.
