# ADR 0003: CLI Parity for Intelligence Tools

- Status: Accepted
- Date: 2025-12-22

## Context

The MCP server exposes `resolve-dependencies` and a set of intelligence tools
for smart recommendations, project analysis, and skill creation. The CLI did
not provide equivalent commands, which limited automation and made the
documentation feel inconsistent for users who prefer direct CLI workflows.

## Decision

Add CLI subcommands that mirror the MCP tool names:

- `resolve-dependencies`
- `recommend-skills-smart`
- `analyze-project-context`
- `suggest-new-skills`
- `create-skill`
- `search-skills-github`

Provide `--format text|json` output for the new commands to align with other CLI
tools. Add the `sync-from-claude` CLI alias so the CLI and MCP naming match for
the sync workflow.

## Alternatives Considered

1. Document MCP-only tools without CLI parity.
   - Rejected because it blocks scripted workflows and surprises CLI users.
2. Add a generic "mcp" CLI wrapper for tool invocation.
   - Rejected because it hides typed flags and complicates discoverability.
3. Expose only a subset of intelligence tools via CLI.
   - Rejected because it fragments the feature surface and documentation.

## Consequences

- CLI users can access intelligence and dependency tooling without an MCP client.
- The CLI surface grows, increasing documentation and test obligations.
- Output formats must remain stable or be versioned as the API evolves.

## Metadata

- Author: Codex
- Related: README.md, book/src/cli.md, docs/architecture.md
