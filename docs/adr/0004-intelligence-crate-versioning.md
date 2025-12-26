# ADR 0004: Intelligence Crate Independent Versioning

- Status: Accepted
- Date: 2025-12-23

## Context

The `skrills-intelligence` crate provides context-aware recommendations, project
analysis, usage analytics, and skill creation helpers. Unlike other workspace
crates, its API surface evolves more rapidly as we experiment with recommendation
algorithms and project context features.

During the v0.3.x release cycle, the intelligence crate underwent significant
API changes (new `RecommendationScorer`, `SkillGapAnalysis`, GitHub search,
LLM generation) that would have forced multiple breaking changes to the entire
workspace if we kept versions synchronized.

## Decision

Allow `skrills-intelligence` to version independently from other workspace crates:

1. **Independent minor/patch versions**: The intelligence crate may advance to
   0.4.x, 0.5.x, etc. while other crates remain at 0.3.x.

2. **Pinned dependencies**: `skrills-server` pins the exact intelligence version
   in `Cargo.toml` (e.g., `version = "0.4.0"`) to prevent accidental upgrades.

3. **Synchronized major versions**: When the workspace reaches 1.0, all crates
   will synchronize major versions for stability guarantees.

4. **Changelog separation**: The intelligence crate maintains its own changelog
   section in `CHANGELOG.md` when its changes don't affect other crates.

## Rationale

- **Experimentation freedom**: Recommendation algorithms and project analysis
  are still evolving. Independent versioning allows rapid iteration.

- **Stability for consumers**: Other crates (`server`, `cli`) expose stable
  public APIs. Keeping them at consistent versions signals stability.

- **Reduced churn**: Breaking changes in intelligence types don't force
  version bumps across the entire workspace.

## Alternatives Considered

1. **Lock-step versioning**: All crates share the same version.
   - Rejected: Forces unnecessary version bumps when only intelligence changes.

2. **Separate repository**: Extract intelligence to its own repo.
   - Rejected: Increases maintenance burden and complicates integration testing.

3. **Feature flags for experimental APIs**: Use `#[cfg(feature = "unstable")]`.
   - Rejected: Adds complexity; independent versioning is simpler.

## Consequences

### Positive
- Intelligence features can iterate faster without blocking other releases.
- Clearer signal to users about which parts of the API are stable.

### Negative
- Slightly more complex version management in `Cargo.toml`.
- Must ensure `server` tests pass when intelligence version changes.

## Related

- [ADR 0003: CLI Parity for Intelligence Tools](0003-cli-parity-intelligence-tools.md)
- `crates/intelligence/CHANGELOG.md` (if created)
