# Frequently Asked Questions

### Why did the installer URL with `/main/` fail?

The installer URL with `/main/` can fail if the repository's default branch is not named `main`. To avoid this issue, we use the branch-agnostic `/HEAD/` path in the installer URL: `https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh`. This special path automatically resolves to the latest commit on the default branch.

### Which release asset should I download manually?

Select the archive that matches your system's architecture (target triple). For example, for a 64-bit Linux system, download `skrills-x86_64-unknown-linux-gnu.tar.gz`. After extracting, you'll find the `skrills` binary at the root.

### How do I resolve the `MCP startup failed: missing field "type"` error in Codex?

This error indicates the MCP server registration is missing `type = "stdio"`. Reinstall with the latest installer (`install.sh` or `install.ps1`).

For manual fix:
1. Add `type: "stdio"` to the `skrills` entry in `~/.codex/mcp_servers.json`
2. Add `type = "stdio"` under `[mcp_servers."skrills"]` in `~/.codex/config.toml`
3. Restart Codex

Run `skrills doctor` to verify configuration.

### Does this project replace my existing Claude skills?

No. Skrills is non-destructive. It reads skills from their default locations and can mirror them between Claude and Codex. It won't overwrite files unless you explicitly run sync commands (and even then, `--skip-existing-commands` protects local customizations).

### How is this project different from other skill management tools?

Skrills focuses on skill quality and cross-CLI portability:
- **Validation**: Validates skills against Claude Code (permissive) and Codex CLI (strict) requirements
- **Analysis**: Identifies optimization opportunities based on token usage
- **Bidirectional Sync**: Keeps configurations in sync between Claude Code and Codex CLI

See the [Comparison](../book/src/comparison.md) for details.

### How do I validate skills for Codex compatibility?

```bash
skrills validate --target codex              # Check Codex compatibility
skrills validate --target codex --autofix    # Auto-add missing frontmatter
```

### How do I build the documentation locally?

Use the `Makefile`:
- `make book` - Build and open the mdBook
- `make book-serve` - Live-reloading as you edit
- `make docs` - Generate Rust API documentation

### Does the system work offline?

Yes. As long as you have the `skrills` binary and skills stored locally, both the MCP server and CLI work without internet access.

### What are the security considerations?

Skrills is designed with security in mind:
- Server communicates over standard I/O
- Operates with minimal file access privileges
- No bundled secrets
- You control which skill directories are exposed

Always review third-party skills before using them.
