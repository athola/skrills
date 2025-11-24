# Release artifact naming

Install scripts select a release asset whose filename contains the target triple. Publish archives accordingly:

```
codex-mcp-skills-<target>.tar.gz
```

Where `<target>` is one of:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

Inside the archive, include the binary named `codex-mcp-skills` (or `codex-mcp-skills.exe` on Windows) at the archive root.

Default release repo: `athola/codex-mcp-skills`. Override with env `CODEX_SKILLS_GH_REPO` if needed.
