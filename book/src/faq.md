# Frequently Asked Questions (extended)

### Why did the installer URL with `/main/` 404?
The repository default branch is `master`. Use `/HEAD/` in the raw URL so it resolves regardless of the default branch:
```
curl -LsSf https://raw.githubusercontent.com/${CODEX_SKILLS_GH_REPO:-athola/codex-mcp-skills}/HEAD/scripts/install.sh | sh
```

### Which release asset maps to my machine?
Check your Rust/Cargo target triple (e.g., `rustc -vV | grep host`). Download the archive whose filename ends with that triple, such as `codex-mcp-skills-x86_64-apple-darwin.tar.gz`. Windows builds include `.exe` inside.

### How is this different from other public skill efforts?
Most alternatives fall into a few buckets: static skill bundles for manual copy, CI pipelines that render SKILL-like docs into prompts, shared rule repositories, local-only sync CLIs, or tutorials. codex-mcp-skills adds an MCP server, Codex hook, cross-agent sync, and turnkey installers so skills become runtime resources.

### Can I keep Claude and Codex skills in sync automatically?
Yes. Use `codex-mcp-skills sync` to mirror Claude skills into Codex paths, and the autoload hook will surface them on prompt submission.

### Does the MCP server expose everything on disk?
No. It only reads directories you point it to (`--skill-dir` flags or defaults). Use separate paths for trusted vs. untrusted skills and avoid passing sensitive files.

### How do I contribute new skills?
Add them to your skills directory and rerun `codex-mcp-skills list` to verify discovery. For upstream contribution, follow the repo’s PR process and include tests or sample prompts when relevant.
