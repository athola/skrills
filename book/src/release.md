# Release & Distribution

- **Targets**: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`,
  `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`,
  `aarch64-pc-windows-msvc`.
- **Asset naming**: `codex-mcp-skills-<target>.tar.gz` with the binary at the
  archive root (`codex-mcp-skills` or `.exe`).
- **Installers**: curl/PowerShell scripts auto-select the matching asset via the
  GitHub API. Default repo `athola/codex-mcp-skills`; override with
  `CODEX_SKILLS_GH_REPO`.
- **CI**: GitHub Actions build and upload the assets on `v*` tags; mdBook is
  deployed to GitHub Pages.
- **Docs**: `make docs` (cargo doc) and `make book` (mdBook, opens locally).
  Live reload via `make book-serve`.
- **Features**: `watch` (default) enables filesystem watching; minimal builds
  use `--no-default-features` or `make build-min`.

For maintainer notes on artifact layout, see `docs/release-artifacts.md`.
