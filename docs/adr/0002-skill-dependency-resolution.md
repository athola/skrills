# ADR 0002: Skill Dependency Resolution

- Status: Accepted
- Date: 2025-12-15
- Issue: #20

## Context

Skills in Claude Code and Codex CLI often build on shared functionality. Without a dependency mechanism, skill authors must either duplicate content or manually ensure prerequisites are loaded. This creates maintenance burden and risks inconsistent behavior when base skills change.

## Decision

Implement a dependency tracking and resolution system with these key choices:

1. **Declaration syntax**: Support three forms in YAML frontmatter:
   - Simple: `base-skill` (any version, any source)
   - Compact: `codex:auth-helpers@^1.0` (source:name@version)
   - Structured: `{ name, version, source, optional }` (explicit fields)

2. **Resolution behavior**: Return list of URIs with metadata; fall back to concatenated content if client can't reliably load URIs.

3. **Version constraints**: Use semver from the start to avoid migration pain later.

4. **Optional dependencies**: Include warning when skipped; configurable via `strict_optional` flag to treat missing optionals as errors.

5. **Source pinning**: Allow `source:skill-name` syntax to disambiguate skills with identical names across sources.

## Consequences

- Skills can now declare dependencies, enabling modular skill design.
- Resolution adds latency on first load (mitigated by caching).
- Circular dependency detection prevents infinite loops but may reject valid use cases requiring refactoring.
- Semver constraints require skill authors to maintain version numbers.
- Implementation adds ~800 lines across validate, analyze, and server crates.

## References

- Full specification: [docs/dependency-resolution.md](../dependency-resolution.md)
- Implementation: `crates/analyze/src/resolve.rs`
