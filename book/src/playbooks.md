# Playbooks and Demo Scripts

These scripted walkthroughs are for team training and validating new builds. They are from the demo scripts in [`docs/`](docs/) and the design spikes in [`docs/plans/`](docs/plans/).

## Claude Code (Hook Path)

- **Installation**: Install `skrills` with Claude hook support: `./scripts/install.sh --client claude`.
- **Verification**: Confirm the `prompt.on_user_prompt_submit` hook and `skrills` MCP server are registered.
- **Demo Flow**: Start the demo by asking Claude to list active hooks, then list skills with `list-skills`. Then, use prompts to trigger autoloading (e.g., requests for TDD guidance or debugging flaky tests). Finish by displaying the `autoload-snippet` output to show the dynamically matched skills.

## Design Spikes for Future Consideration

- **[Skill Autoload via MCP](docs/plans/2025-11-22-skill-autoload-mcp.md) (2025-11-22)**: Focuses on unified autoload, syncing from `~/.claude`, enforcing `max-bytes` limits, and hook-friendly JSON output.
- **[Modular Workspaces](docs/plans/2025-11-25-modular-workspaces.md) (2025-11-25)**: Splits crates by responsibility for a thin CLI and reusable core logic.

These design spikes provide foundational insights into the current architecture. Review them thoroughly before changing autoloading behavior.