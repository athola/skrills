# Design: Skill Dependency Resolution (Issue #20)

## Summary

Implement a dependency tracking and resolution system for skills that allows:
- Declaring dependencies via YAML frontmatter
- Automatic resolution with circular dependency detection
- Semver version constraints
- Source pinning (e.g., `codex:base-skill`)
- Optional dependency handling with configurable behavior

## User Decisions

1. **Resolution behavior**: Return list of URIs (B) + metadata (C); fall back to concatenated content (A) if client can't reliably load URIs
2. **Optional dependencies**: Include warning when skipped; configurable to NOT skip via explicit flag
3. **Version constraints**: Support semver now
4. **Source pinning**: Allow `source:skill-name` syntax

## Frontmatter Schema

```yaml
---
name: my-skill
description: Does something useful
version: 1.2.0  # Skill's own version
depends:
  - base-skill                           # Simple: any version, any source
  - name: utility-skill                  # Structured: explicit name
    version: "^2.0"                      # Semver constraint
    source: codex                        # Source pinning
    optional: true                       # Optional dependency
  - codex:auth-helpers@^1.0              # Compact: source:name@version
---
```

## Data Structures

### In `crates/validate/src/frontmatter.rs`

```rust
/// A declared skill dependency.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum DeclaredDependency {
    /// Simple string form: "skill-name" or "source:skill-name@version"
    Simple(String),
    /// Structured form with explicit fields
    Structured {
        name: String,
        #[serde(default)]
        version: Option<String>,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        optional: bool,
    },
}

/// Normalized dependency after parsing.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedDependency {
    pub name: String,
    pub version_req: Option<semver::VersionReq>,
    pub source: Option<SkillSource>,
    pub optional: bool,
}

/// Extended frontmatter with dependency support.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub depends: Vec<DeclaredDependency>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}
```

### In `crates/analyze/src/resolve.rs` (new file)

```rust
/// Resolution options.
#[derive(Debug, Clone, Default)]
pub struct ResolveOptions {
    /// If true, treat missing optional deps as errors.
    pub strict_optional: bool,
    /// Maximum recursion depth (default: 50).
    pub max_depth: usize,
}

/// A resolved dependency with metadata.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedDependency {
    pub uri: String,
    pub name: String,
    pub source: SkillSource,
    pub version: Option<semver::Version>,
    pub optional: bool,
    pub depth: usize,
}

/// Resolution result.
#[derive(Debug, Clone, Serialize)]
pub struct ResolutionResult {
    /// Resolved dependencies in topological order (deps first).
    pub resolved: Vec<ResolvedDependency>,
    /// Warnings (e.g., skipped optional deps).
    pub warnings: Vec<String>,
    /// Whether resolution was successful.
    pub success: bool,
}

/// Errors during resolution.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ResolveError {
    #[error("Circular dependency detected: {0}")]
    CircularDependency(String),
    #[error("Dependency not found: {name} (required by {required_by})")]
    NotFound { name: String, required_by: String },
    #[error("Version mismatch: {name} requires {required} but found {found}")]
    VersionMismatch { name: String, required: String, found: String },
    #[error("Max resolution depth exceeded: {0}")]
    MaxDepthExceeded(usize),
}
```

## Resolution Algorithm

```
resolve(skill, registry, options) -> Result<ResolutionResult>:
    visited = Set()
    in_stack = Set()  # For cycle detection
    resolved = []
    warnings = []

    fn visit(skill_name, depth, required_by):
        if skill_name in in_stack:
            return Err(CircularDependency)
        if skill_name in visited:
            return Ok(())  # Already resolved
        if depth > options.max_depth:
            return Err(MaxDepthExceeded)

        in_stack.add(skill_name)
        skill = registry.lookup(skill_name)

        if skill is None:
            if is_optional:
                warnings.push("Skipped optional: {skill_name}")
                return Ok(())
            return Err(NotFound)

        # Check version constraint
        if version_req and skill.version:
            if !version_req.matches(skill.version):
                return Err(VersionMismatch)

        # Recursively resolve dependencies
        for dep in skill.depends:
            visit(dep.name, depth + 1, skill_name)?

        in_stack.remove(skill_name)
        visited.add(skill_name)
        resolved.push(skill)  # Post-order = deps before dependents

    visit(skill, 0, "root")
    return ResolutionResult { resolved, warnings, success: true }
```

## MCP Integration

### Resource Read Enhancement

When `read_resource(uri)` is called:

1. Parse URI, lookup skill
2. Check for `?resolve=true` query parameter or header
3. If resolving:
   - Run resolution algorithm
   - Return JSON with `resolved_uris`, `warnings`, and skill content
4. If not resolving:
   - Return skill content as before (backward compatible)

### New Tool: `resolve-dependencies`

```json
{
  "name": "resolve-dependencies",
  "description": "Resolve skill dependencies and return load order",
  "inputSchema": {
    "type": "object",
    "properties": {
      "skill": { "type": "string", "description": "Skill name or URI" },
      "strict_optional": { "type": "boolean", "default": false },
      "include_content": { "type": "boolean", "default": false }
    },
    "required": ["skill"]
  }
}
```

**Response format:**
```json
{
  "success": true,
  "resolved": [
    { "uri": "skill://skrills/codex/base-skill", "name": "base-skill", "depth": 2 },
    { "uri": "skill://skrills/codex/utility", "name": "utility", "depth": 1 },
    { "uri": "skill://skrills/codex/my-skill", "name": "my-skill", "depth": 0 }
  ],
  "warnings": ["Skipped optional dependency: optional-helper"],
  "content": "..." // Only if include_content=true (concatenated)
}
```

## Validation Updates

### In `crates/validate/src/common.rs`

New validation issues:
- `DependencyNotFound { name, source }` - Referenced dependency doesn't exist
- `CircularDependency { chain }` - Circular reference detected
- `InvalidVersionConstraint { dep, error }` - Malformed semver
- `VersionMismatch { dep, required, found }` - Constraint not satisfied
- `InvalidDependencyFormat { raw }` - Can't parse dependency string

### In `validate-skills` tool

Add `--check-deps` flag to enable dependency validation during skill scanning.

## Implementation Phases

### Phase 1: Schema Extension (~150 lines)
- Add `DeclaredDependency` enum to frontmatter.rs
- Add `version`, `depends` fields to `SkillFrontmatter`
- Add parsing tests

### Phase 2: Semver Support (~100 lines)
- Add semver to validate crate dependencies
- Implement `NormalizedDependency` with version parsing
- Add `parse_dependency()` function for compact syntax
- Add validation for version constraints

### Phase 3: Resolution Engine (~300 lines)
- Create `crates/analyze/src/resolve.rs`
- Implement `DependencyResolver` struct
- Add cycle detection via in-stack tracking
- Add memoization cache
- Implement topological sort

### Phase 4: MCP Read Integration (~150 lines)
- Extend `read_resource()` with resolution option
- Add response format for resolved dependencies
- Implement concatenation fallback mode

### Phase 5: MCP Tool (~100 lines)
- Add `resolve-dependencies` tool definition
- Implement tool handler
- Add JSON response formatting

### Phase 6: Validation Integration (~100 lines)
- Add dependency validation issues
- Implement `--check-deps` flag
- Add validation during cache refresh (optional)

## Testing Strategy

### Unit Tests
- Frontmatter parsing (simple, structured, compact syntax)
- Semver constraint matching
- Cycle detection (A→B→A, A→B→C→A)
- Topological ordering
- Version mismatch detection

### Integration Tests
- Create test skill fixtures with dependencies
- Test MCP resource read with resolution
- Test `resolve-dependencies` tool
- Test validation with missing deps

## Backward Compatibility

- Skills without `depends` field work unchanged
- `extra` HashMap continues to capture unknown fields
- Resolution is opt-in (query param or tool)
- No changes to existing URIs or resource format

## Files to Modify

1. `crates/validate/src/frontmatter.rs` - Schema extension
2. `crates/validate/src/common.rs` - New validation issues
3. `crates/validate/Cargo.toml` - Add semver dependency
4. `crates/analyze/src/resolve.rs` - New resolution module
5. `crates/analyze/src/lib.rs` - Export resolve module
6. `crates/server/src/app.rs` - MCP integration
7. `crates/discovery/src/types.rs` - Possibly extend SkillMeta

## Open for Future

- Transitive version conflict resolution (pick highest compatible)
- Dependency lockfiles
- Remote skill registry integration
- Dependency visualization tool
