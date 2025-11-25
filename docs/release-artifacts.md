# Release Artifact Naming and Structure

This document details the naming conventions and internal structure for release artifacts. These guidelines ensure that installation scripts can correctly identify and extract the appropriate assets for each target platform.

Install scripts select a release asset whose filename contains the target triple. Therefore, archives should be published following this pattern:

```
codex-mcp-skills-<target>.tar.gz
```

Where `<target>` is one of the supported target triples:
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`
- `aarch64-pc-windows-msvc`

Inside each archive, the binary must be located at the archive root and named `codex-mcp-skills` (or `codex-mcp-skills.exe` for Windows builds).

The default release repository is `athola/codex-mcp-skills`. This can be overridden by setting the `CODEX_SKILLS_GH_REPO` environment variable if necessary.
