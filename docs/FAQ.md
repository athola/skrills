# FAQ

**Q: The installer URL with `/main/` failed. What URL should I use?**  
Use the branch-agnostic path: `https://raw.githubusercontent.com/${CODEX_SKILLS_GH_REPO:-athola/codex-mcp-skills}/HEAD/scripts/install.sh` (or `install.ps1` on Windows). This works even though the default branch is `master`.

**Q: Which release asset should I download manually?**  
Pick the archive whose filename includes your target triple, e.g., `codex-mcp-skills-x86_64-unknown-linux-gnu.tar.gz`. Each archive contains the matching binary at the root.

**Q: Does this replace my existing Claude skills directory?**  
No. The MCP server reads skills from the default locations and can mirror Claude skills; it does not overwrite existing files unless you run sync commands that copy skills on purpose.

**Q: How is this different from other skill efforts?**  
Many efforts either ship static skill bundles, render docs during CI, or provide local-only sync tools. codex-mcp-skills adds an MCP server, Codex hook, and cross-agent sync so skills become runtime resources, not just files.

**Q: How do I build the docs locally?**  
Use `make book` to build and open the mdBook; `make book-serve` live-reloads at `http://localhost:3000`. Use `make docs` for Rust API docs.

**Q: Does it run offline?**  
Yes. Once the binary and skills are present locally, the MCP server and CLI run without network access.

**Q: What about security?**  
The server runs over stdio with least-privilege file reads. No secrets are bundled, and you choose which skill directories to expose. Always review third-party skills before syncing them.

For deeper answers and advanced scenarios, see the book FAQ.
