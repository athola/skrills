# Frequently Asked Questions

### Why does the installer URL with `/main/` sometimes fail?

The installer URL using `/main/` can fail if the repository's default branch is not named `main`. To avoid this issue, use `/HEAD/` in the raw URL:

```bash
curl -LsSf https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh | sh
```

### How do I identify the correct release asset for my system?

To find the correct release asset, determine your system's Rust/Cargo target triple (e.g., by running `rustc -vV | grep host`). Then, download the archive whose filename ends with that specific triple, for instance, `skrills-x86_64-apple-darwin.tar.gz`. Windows builds will have an `.exe` executable inside the archive.

### How can I resolve the `MCP startup failed: missing field "type"` error in Codex?

Re-execute the installer (`install.sh` or `install.ps1`). The updated installer will automatically register `skrills` with `type = "stdio"` in both `~/.codex/mcp_servers.json` and `~/.codex/config.toml`.

**Manual fix**: Add `type: "stdio"` to the `skrills` entry in `mcp_servers.json` and `type = "stdio"` under `[mcp_servers."skrills"]` in `config.toml`. Then, restart Codex. Run `skrills doctor` to confirm that both files are correctly configured.

If the error persists with a third-party MCP server, consider proxying problematic servers through a schema normalizer like `codex-mcp-wrapper`.

### How does `skrills` compare to other skill management tools?

`skrills` focuses on skill quality and portability. It checks skills for compatibility across both Claude Code and Codex CLI, analyzes token usage to identify optimization opportunities, and keeps configurations in sync between the two environments. For a detailed comparison, see the [Project Comparison](./comparison.md).

### Is it possible to automatically synchronize skills between Claude and Codex?

Yes. Use `skrills sync-all` to sync everything (skills, commands, MCP servers, preferences):

```bash
skrills sync-all --from claude --skip-existing-commands
```

Add `--skip-existing-commands` to preserve local command files. The sync is byte-for-byte, so non-UTF-8 commands are preserved.

### How do I validate skills for Codex compatibility?

Use the `validate` command:

```bash
skrills validate --target codex              # Check Codex compatibility
skrills validate --target codex --autofix    # Auto-add missing frontmatter
```

### Does the MCP server expose all content from disk?

No. The MCP server only reads from configured directories, set either via `--skill-dir` flags or through default discovery paths. Use separate paths for trusted and untrusted skills.

### What is the process for contributing new skills?

To contribute, add your new skills to your directory (`~/.claude/skills/` or `~/.codex/skills/`) and run `skrills validate --target both` to verify compatibility. For upstream contributions, follow the repository's PR process.

### Does the system work offline?

Yes. As long as you have the `skrills` binary and your skills stored locally, both the MCP server and CLI will function without an internet connection.

### What are the security considerations?

Skrills is designed with security in mind. The server communicates over standard I/O and operates with minimal file access privileges. It does not bundle any secrets, and you retain control over which skill directories are exposed. Always review third-party skills before using them.
