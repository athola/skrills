# Frequently Asked Questions

### Why does the installer fail with `/main/`?

The installer needs `/HEAD/` to resolve the default branch if it isn't named `main`. Use `https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh`.

### Which release asset should I download?

Download the archive matching your system architecture (e.g., `skrills-x86_64-unknown-linux-gnu.tar.gz`). The binary is at the root.

### How do I fix the `MCP startup failed: missing field "type"` error in Codex?

The MCP server registration is missing `type = "stdio"`. Reinstall with the latest installer, or fix manually:

1. Add `type: "stdio"` to `skrills` in `~/.codex/mcp_servers.json`
2. Add `type = "stdio"` to `[mcp_servers."skrills"]` in `~/.codex/config.toml`
3. Restart Codex.

Run `skrills doctor` to verify.

### Will this overwrite my existing skills?

No. Skrills mirrors skills between Claude and Codex but only overwrites files if you explicitly run `sync` commands (and use `--skip-existing-commands` to protect local changes).

### How is Skrills different from other tools?

Skrills focuses on validation and portability:
- **Validation**: Checks compatibility for Claude Code (permissive), Codex CLI (strict), and Copilot CLI (strict).
- **Analysis**: Reports token usage.
- **Sync**: Keeps configurations consistent across CLIs.

### How do I validate skills for Codex compatibility?

```bash
skrills validate --target codex              # Check Codex compatibility
skrills validate --target codex --autofix    # Auto-add missing frontmatter
```

### How do I build documentation locally?

Use `make`:
- `make book`: Build and open the mdBook
- `make book-serve`: Live-reload
- `make docs`: Generate Rust API docs

### Does it work offline?

Yes. The MCP server and CLI work locally without internet access.

### Why doesn't Copilot have slash commands?

GitHub Copilot CLI uses **Skills** (reusable instructions) and **Agents** (autonomous actors) instead of slash commands.

When syncing from Claude:
- **Skills**: Sync normally.
- **Commands**: Skipped (no equivalent).
- **Agents**: Transformed automatically (`model`/`color` removed, `target: github-copilot` added).

To create a reusable prompt in Copilot, use an agent:

```yaml
# ~/.copilot/agents/my-prompt.agent.md
---
name: my-prompt
description: Does something useful
target: github-copilot
---

Your prompt instructions here...
```

### Security considerations?

Skrills operates with standard I/O and no bundled secrets. You control exposed skill directories. always review third-party skills.
