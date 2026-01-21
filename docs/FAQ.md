# Frequently Asked Questions

### Why did the installer URL with `/main/` fail?

The installer fails if the repository's default branch isn't named `main`. Use the `/HEAD/` path to automatically resolve the latest commit on the default branch: `https://raw.githubusercontent.com/${SKRILLS_GH_REPO:-athola/skrills}/HEAD/scripts/install.sh`.

### Which release asset should I download manually?

Download the archive for your system's architecture (e.g., `skrills-x86_64-unknown-linux-gnu.tar.gz`). The binary is at the root of the archive.

### How do I resolve the `MCP startup failed: missing field "type"` error in Codex?

The MCP server registration is missing `type = "stdio"`. Reinstall with the latest installer, or fix manually:

1. Add `type: "stdio"` to the `skrills` entry in `~/.codex/mcp_servers.json`
2. Add `type = "stdio"` under `[mcp_servers."skrills"]` in `~/.codex/config.toml`
3. Restart Codex

Run `skrills doctor` to verify.

### Does this project replace my existing Claude skills?

No. Skrills is non-destructive. It mirrors skills between Claude and Codex and only overwrites files if you explicitly run `sync` commands (and `--skip-existing-commands` prevents overwriting local changes).

### How is this project different from other skill management tools?

Skrills prioritizes validation and portability:
- **Validation**: Checks skills against Claude Code (permissive) and Codex CLI (strict) requirements.
- **Analysis**: Reports token usage to help optimize context.
- **Bidirectional Sync**: Keeps configurations consistent between CLIs.

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

Yes. The MCP server and CLI work without internet access if the binary and skills are local.

### Why doesn't Copilot have slash commands like Claude or Codex?

GitHub Copilot CLI uses a different architectural paradigm. Instead of slash commands (`/command-name`), Copilot has:

1. **Skills** (`~/.copilot/skills/<name>/SKILL.md`) - Reusable instruction sets that extend capabilities (same format as Codex)
2. **Agents** (`~/.copilot/agents/*.md`) - Autonomous actors with defined tools, targets, and behaviors

When syncing from Claude to Copilot:
- **Skills**: Sync normally (compatible formats)
- **Commands**: Skipped (no equivalent in Copilot)
- **Agents**: Sync with format transformation (Claude's `model`/`color` → Copilot's `target: github-copilot`)

If you want command-like reusable prompts in Copilot, create an agent instead:

```yaml
# ~/.copilot/agents/my-prompt.agent.md
---
name: my-prompt
description: Does something useful
target: github-copilot
---

Your prompt instructions here...
```

The agent paradigm is more powerful but fundamentally different - agents have defined tool access and autonomous behavior rather than being simple prompt templates.

### What are the security considerations?

Skrills operates with minimal privileges over standard I/O and has no bundled secrets. You control which skill directories are exposed. Always review third-party skills before use.
