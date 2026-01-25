# Skill Dependency Resolution

Skills declare dependencies on other skills via YAML frontmatter. The resolution engine handles circular dependency detection, semver version constraints, source pinning, and optional dependencies.

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

### Dependency Formats

| Format | Example | Use Case |
|--------|---------|----------|
| Simple | `base-skill` | Any version, any source |
| Compact | `codex:auth@^1.0` | Pinned source and version |
| Structured | `{ name, version, source, optional }` | Full control |

## Resolution Algorithm

The resolver performs a depth-first traversal with cycle detection:

1. **Cycle detection**: Tracks in-progress nodes and errors if a node is revisited.
2. **Version matching**: Validates semver constraints against skill versions.
3. **Optional handling**: Skips missing optionals with a warning (or errors if `strict_optional` is set).
4. **Topological order**: Returns dependencies before dependents.

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

## MCP Tool: `resolve-dependencies`

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
    { "uri": "skill://skrills/codex/base-skill/SKILL.md", "name": "base-skill", "depth": 2 },
    { "uri": "skill://skrills/codex/utility/SKILL.md", "name": "utility", "depth": 1 },
    { "uri": "skill://skrills/codex/my-skill/SKILL.md", "name": "my-skill", "depth": 0 }
  ],
  "warnings": ["Skipped optional dependency: optional-helper"],
  "content": "..." // Only if include_content=true (concatenated)
}
```

## Validation

The `validate-skills` tool reports dependency issues when the `--check-deps` flag is enabled:

| Issue | Severity | Description |
|-------|----------|-------------|
| `DependencyNotFound` | Error | Referenced skill doesn't exist |
| `CircularDependency` | Error | Cycle detected in dependency graph |
| `InvalidVersionConstraint` | Error | Malformed semver string |
| `VersionMismatch` | Error | Constraint not satisfied |
| `InvalidDependencyFormat` | Error | Can't parse dependency string |

## Implementation Details

### Data Structures

#### Frontmatter Schema (`crates/validate/src/frontmatter.rs`)

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

#### Resolution API (`crates/analyze/src/resolve.rs`)

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

### MCP Resource Read Enhancement

When `read_resource(uri)` is called with dependency resolution:

1. Parse URI, lookup skill.
2. Check for `?resolve=true` query parameter or header.
3. If resolving:
   - Run resolution algorithm.
   - Return JSON with `resolved_uris`, `warnings`, and skill content.
4. If not resolving:
   - Return skill content as before.
