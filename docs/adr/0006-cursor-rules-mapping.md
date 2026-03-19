# ADR 0006: Cursor Rules Mapping Strategy

- Status: Accepted
- Date: 2026-03-18

## Context

Cursor IDE stores project-level instructions as `.mdc` (markdown-config) files
in `.cursor/rules/` with YAML frontmatter that controls *when* the rule is
applied. This differs from Claude (single `CLAUDE.md`), Copilot
(`*.instructions.md`), and Codex (`config.toml` instructions section).

The sync adapter needs to map between these formats without losing intent.

## Decision

Map Cursor rules through the `instructions` field of the `AgentAdapter` trait,
using a heuristic mode-derivation strategy:

| Source | Cursor Rule Mode | Frontmatter |
|--------|-----------------|-------------|
| `CLAUDE.md` or `claude-instructions` | Always-apply | `alwaysApply: true` |
| Source with `globs:` in frontmatter | Auto-attach | Preserve globs, `alwaysApply: false` |
| Source with `alwaysApply: true` | Always-apply | Preserve as-is |
| Other instructions | Agent-requested | `description: "Rule: {name}"`, `alwaysApply: false` |

### Bidirectional behavior

- **Claude to Cursor**: Instructions become `.mdc` files with derived mode.
- **Cursor to Claude**: `.mdc` files (and `.md` files in `rules/`) are read as
  instructions with frontmatter preserved in content bytes.

### Reuse of `instructions` trait method

The `AgentAdapter::read_instructions()` / `write_instructions()` methods serve
as the generic carrier. Each adapter interprets "instructions" in its native
format:

- Claude: `CLAUDE.md`
- Copilot: `*.instructions.md`
- Cursor: `.cursor/rules/*.mdc`

This polymorphic reuse avoids adding a Cursor-specific trait method.

## Alternatives Considered

1. **Add `read_rules()` / `write_rules()` to `AgentAdapter`**.
   Rejected: adds methods only one adapter implements, violates ISP for all others.

2. **Store rules as commands instead of instructions**.
   Rejected: semantic mismatch — rules are project-level guidance, not
   user-invoked actions.

3. **Always use `alwaysApply: true`**.
   Rejected: loses granularity for glob-scoped and agent-requested rules.

## Consequences

- Name-based heuristic (`claude`, `claude-instructions`) for always-apply mode
  is fragile but covers the primary use case.
- Cursor-only frontmatter fields (`globs`, `alwaysApply`) have no Claude
  equivalent; they are preserved in content bytes during Cursor-to-Claude sync
  but have no effect in Claude.
- Future adapters with rule-like concepts can follow the same
  instructions-based mapping pattern.

## References

- Implementation: `crates/sync/src/adapters/cursor/rules.rs`
- Trait definition: `crates/sync/src/adapters/traits.rs`
