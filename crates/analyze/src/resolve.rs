//! Dependency resolution for skills.
//!
//! Provides two resolution strategies:
//! - `DependencyResolver`: Trait-based, on-demand resolution (flexible)
//! - `DependencyGraph`: Pre-computed, index-based resolution (fast)
//!
//! # Performance
//!
//! For best performance, use `DependencyGraph` which:
//! - Pre-computes the entire graph at build time: O(V + E)
//! - Caches resolution results: O(1) for repeated lookups
//! - Uses index-based adjacency lists: no string hashing during traversal
//!
//! # Example
//!
//! ```rust
//! use skrills_analyze::resolve::{DependencyGraph, GraphBuilder, SkillInfo};
//! use skrills_discovery::SkillSource;
//! use skrills_validate::frontmatter::{DeclaredDependency, SkillFrontmatter};
//!
//! let base = SkillInfo {
//!     name: "base".into(),
//!     source: SkillSource::Extra(0),
//!     uri: "skill://base".into(),
//!     version: None,
//!     frontmatter: Some(SkillFrontmatter {
//!         name: Some("base".into()),
//!         ..Default::default()
//!     }),
//! };
//!
//! let child = SkillInfo {
//!     name: "child".into(),
//!     source: SkillSource::Extra(0),
//!     uri: "skill://child".into(),
//!     version: None,
//!     frontmatter: Some(SkillFrontmatter {
//!         name: Some("child".into()),
//!         depends: vec![DeclaredDependency::Simple("base".into())],
//!         ..Default::default()
//!     }),
//! };
//!
//! let graph = GraphBuilder::new()
//!     .add_skill(base)
//!     .add_skill(child)
//!     .build()
//!     .expect("graph should build");
//!
//! let resolved = graph.resolve("child").expect("resolution succeeds");
//! assert_eq!(resolved.resolved.len(), 2);
//! ```

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use skrills_discovery::SkillSource;
use skrills_validate::frontmatter::SkillFrontmatter;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use thiserror::Error;

// ============================================================================
// Error Types
// ============================================================================

/// Errors that can occur during dependency resolution.
#[derive(Debug, Clone, Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// A circular dependency was detected.
    #[error("Circular dependency detected: {chain}")]
    CircularDependency {
        /// The cycle chain as a readable string (e.g., "A -> B -> C -> A").
        chain: String,
    },

    /// A required dependency was not found in the registry.
    #[error("Dependency not found: '{name}' (required by '{required_by}')")]
    NotFound {
        /// The missing dependency name.
        name: String,
        /// The skill that requires this dependency.
        required_by: String,
    },

    /// A dependency's version doesn't satisfy the requirement.
    #[error("Version mismatch for '{name}': requires {required} but found {found}")]
    VersionMismatch {
        /// The dependency name.
        name: String,
        /// The required version constraint.
        required: String,
        /// The actual version found.
        found: String,
    },

    /// Multiple skills require incompatible versions.
    #[error("Version conflict for '{name}': {conflicts}")]
    VersionConflict {
        /// The dependency with conflicting requirements.
        name: String,
        /// Description of the conflicts.
        conflicts: String,
    },

    /// Maximum resolution depth exceeded (likely a very deep or infinite chain).
    #[error("Maximum resolution depth ({0}) exceeded")]
    MaxDepthExceeded(usize),

    /// Failed to parse frontmatter or dependencies.
    #[error("Failed to parse dependencies for '{skill}': {message}")]
    ParseError {
        /// The skill that failed to parse.
        skill: String,
        /// Error message.
        message: String,
    },

    /// Skill not found in graph.
    #[error("Skill '{0}' not found in dependency graph")]
    SkillNotInGraph(String),
}

// ============================================================================
// Options and Results
// ============================================================================

/// Options for controlling resolution behavior.
#[derive(Debug, Clone)]
pub struct ResolveOptions {
    /// If true, treat missing optional dependencies as errors.
    pub strict_optional: bool,
    /// Maximum recursion depth (default: 50).
    pub max_depth: usize,
    /// If true, skip version checking.
    pub ignore_versions: bool,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        Self {
            strict_optional: false,
            max_depth: 50,
            ignore_versions: false,
        }
    }
}

/// A resolved dependency with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedDependency {
    /// URI for loading this skill.
    pub uri: String,
    /// Skill name.
    pub name: String,
    /// Source of this skill.
    pub source: SkillSource,
    /// Skill version, if known.
    pub version: Option<String>,
    /// Whether this was an optional dependency.
    pub optional: bool,
    /// Depth in the dependency tree (0 = root).
    pub depth: usize,
}

/// Result of dependency resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[must_use]
pub struct ResolutionResult {
    /// Resolved dependencies in topological order (dependencies first, then dependents).
    pub resolved: Vec<ResolvedDependency>,
    /// Warnings (e.g., skipped optional dependencies).
    pub warnings: Vec<String>,
    /// Whether resolution was fully successful.
    pub success: bool,
}

// ============================================================================
// Skill Info
// ============================================================================

/// Information about a skill in the registry.
#[derive(Debug, Clone)]
pub struct SkillInfo {
    /// Skill name.
    pub name: String,
    /// Source of this skill.
    pub source: SkillSource,
    /// URI for loading this skill.
    pub uri: String,
    /// Parsed version, if available.
    pub version: Option<semver::Version>,
    /// Parsed frontmatter with dependencies.
    pub frontmatter: Option<SkillFrontmatter>,
}

// ============================================================================
// Pre-computed Dependency Graph (Optimized)
// ============================================================================

/// Edge in the dependency graph.
#[derive(Debug, Clone)]
struct Edge {
    /// Target skill index.
    target: usize,
    /// Version requirement, if any.
    version_req: Option<semver::VersionReq>,
    /// Whether this dependency is optional.
    optional: bool,
}

/// Pre-computed, index-based dependency graph for fast resolution.
///
/// Build this once at startup and reuse for all resolution requests.
/// Resolution results are cached for O(1) repeated lookups.
///
/// # Thread Safety
/// The graph can be shared across threads; cache access is guarded by `RwLock`
/// and the underlying skill metadata is immutable after construction.
#[derive(Debug)]
pub struct DependencyGraph {
    /// Skill metadata indexed by position.
    skills: Vec<SkillInfo>,
    /// Name to index mapping (includes source:name variants).
    name_index: HashMap<String, usize>,
    /// Adjacency list: skill index -> edges to dependencies.
    edges: Vec<Vec<Edge>>,
    /// Cached resolution results (skill_idx -> resolved order).
    cache: RwLock<HashMap<usize, Arc<CachedResolution>>>,
    /// Resolution options.
    options: ResolveOptions,
    /// Warnings produced while building the graph (e.g., duplicate skill names).
    build_warnings: Vec<String>,
}

/// Cached resolution result.
#[derive(Debug, Clone)]
struct CachedResolution {
    /// Indices in topological order.
    order: Vec<usize>,
    /// Depth for each index in order.
    depths: Vec<usize>,
    /// Optional flag for each index in order.
    optionals: Vec<bool>,
    /// Warnings generated during resolution.
    warnings: Vec<String>,
}

impl DependencyGraph {
    /// Create a new graph builder.
    pub fn builder() -> GraphBuilder {
        GraphBuilder::new()
    }

    /// Resolve dependencies for a skill by name.
    ///
    /// Returns cached result if available, otherwise computes and caches.
    #[must_use = "resolution result contains important dependency information"]
    pub fn resolve(&self, skill_name: &str) -> Result<ResolutionResult, ResolveError> {
        let idx = self
            .name_index
            .get(skill_name)
            .copied()
            .ok_or_else(|| ResolveError::SkillNotInGraph(skill_name.to_string()))?;

        self.resolve_by_index(idx)
    }

    /// Resolve by skill index (internal, for when index is already known).
    fn resolve_by_index(&self, idx: usize) -> Result<ResolutionResult, ResolveError> {
        // Check cache first (read lock)
        {
            let cache = self.cache.read();
            if let Some(cached) = cache.get(&idx) {
                return Ok(self.build_result(cached));
            }
        }

        // Compute resolution
        let cached = self.compute_resolution(idx)?;
        let result = self.build_result(&cached);

        // Cache result (write lock)
        {
            let mut cache = self.cache.write();
            cache.insert(idx, Arc::new(cached));
        }

        Ok(result)
    }

    /// Compute resolution for a skill (not cached).
    fn compute_resolution(&self, root_idx: usize) -> Result<CachedResolution, ResolveError> {
        let mut visited: HashSet<usize> = HashSet::new();
        let mut in_stack: HashSet<usize> = HashSet::new();
        let mut stack_path: Vec<usize> = Vec::new();
        let mut order: Vec<usize> = Vec::new();
        let mut depths: Vec<usize> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut optionals: Vec<bool> = Vec::new();

        self.visit_index(
            root_idx,
            false,
            0,
            &mut visited,
            &mut in_stack,
            &mut stack_path,
            &mut order,
            &mut depths,
            &mut warnings,
            &mut optionals,
        )?;

        Ok(CachedResolution {
            order,
            depths,
            optionals,
            warnings,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn visit_index(
        &self,
        idx: usize,
        optional: bool,
        depth: usize,
        visited: &mut HashSet<usize>,
        in_stack: &mut HashSet<usize>,
        stack_path: &mut Vec<usize>,
        order: &mut Vec<usize>,
        depths: &mut Vec<usize>,
        warnings: &mut Vec<String>,
        optionals: &mut Vec<bool>,
    ) -> Result<(), ResolveError> {
        // Depth limit
        if depth > self.options.max_depth {
            return Err(ResolveError::MaxDepthExceeded(self.options.max_depth));
        }

        // Cycle detection
        if in_stack.contains(&idx) {
            let cycle_start = stack_path.iter().position(|&i| i == idx).unwrap_or(0);
            let cycle_names: Vec<_> = stack_path[cycle_start..]
                .iter()
                .chain(std::iter::once(&idx))
                .map(|&i| self.skills[i].name.as_str())
                .collect();
            return Err(ResolveError::CircularDependency {
                chain: cycle_names.join(" -> "),
            });
        }

        // Already resolved
        if visited.contains(&idx) {
            return Ok(());
        }

        // Mark in-progress
        in_stack.insert(idx);
        stack_path.push(idx);

        // Visit dependencies
        for edge in &self.edges[idx] {
            // Version check
            if !self.options.ignore_versions {
                if let (Some(req), Some(actual)) =
                    (&edge.version_req, &self.skills[edge.target].version)
                {
                    if !req.matches(actual) {
                        return Err(ResolveError::VersionMismatch {
                            name: self.skills[edge.target].name.clone(),
                            required: req.to_string(),
                            found: actual.to_string(),
                        });
                    }
                }
            }

            // Handle missing (edges only point to valid indices, but check anyway)
            debug_assert!(
                edge.target < self.skills.len(),
                "edge target {} out of bounds (max index: {})",
                edge.target,
                self.skills.len().saturating_sub(1)
            );
            if edge.target >= self.skills.len() {
                if edge.optional && !self.options.strict_optional {
                    warnings.push("Skipped optional dependency (invalid index)".to_string());
                    continue;
                }
                return Err(ResolveError::NotFound {
                    name: format!("index-{}", edge.target),
                    required_by: self.skills[idx].name.clone(),
                });
            }

            let child_optional = optional || edge.optional;
            self.visit_index(
                edge.target,
                child_optional,
                depth + 1,
                visited,
                in_stack,
                stack_path,
                order,
                depths,
                warnings,
                optionals,
            )?;
        }

        // Done processing
        in_stack.remove(&idx);
        stack_path.pop();
        visited.insert(idx);

        // Post-order: add after dependencies
        order.push(idx);
        depths.push(depth);
        optionals.push(optional);

        Ok(())
    }

    /// Build ResolutionResult from cached data.
    fn build_result(&self, cached: &CachedResolution) -> ResolutionResult {
        let resolved = cached
            .order
            .iter()
            .zip(cached.depths.iter())
            .zip(cached.optionals.iter())
            .map(|((&idx, &depth), &optional)| {
                let skill = &self.skills[idx];
                ResolvedDependency {
                    uri: skill.uri.clone(),
                    name: skill.name.clone(),
                    source: skill.source.clone(),
                    version: skill.version.as_ref().map(|v| v.to_string()),
                    optional,
                    depth,
                }
            })
            .collect();

        let mut warnings = self.build_warnings.clone();
        warnings.extend(cached.warnings.clone());

        ResolutionResult {
            resolved,
            warnings,
            success: true,
        }
    }

    /// Get the number of skills in the graph.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Check if the graph is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Iterate over skills in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &SkillInfo> {
        self.skills.iter()
    }

    /// Return the immutable skill list.
    pub fn skills(&self) -> &[SkillInfo] {
        &self.skills
    }

    /// Get a skill by name.
    pub fn get(&self, name: &str) -> Option<&SkillInfo> {
        self.name_index.get(name).map(|&idx| &self.skills[idx])
    }

    /// Clear the resolution cache (call after modifying skills).
    pub fn clear_cache(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> (usize, usize) {
        let cache = self.cache.read();
        (cache.len(), self.skills.len())
    }
}

// ============================================================================
// Graph Builder
// ============================================================================

/// Builder for constructing a DependencyGraph.
#[derive(Debug, Default)]
pub struct GraphBuilder {
    skills: Vec<SkillInfo>,
    options: ResolveOptions,
}

impl GraphBuilder {
    /// Create a new graph builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set resolution options.
    pub fn with_options(mut self, options: ResolveOptions) -> Self {
        self.options = options;
        self
    }

    /// Add a skill to the graph.
    pub fn add_skill(mut self, skill: SkillInfo) -> Self {
        self.skills.push(skill);
        self
    }

    /// Add multiple skills to the graph.
    pub fn add_skills(mut self, skills: impl IntoIterator<Item = SkillInfo>) -> Self {
        self.skills.extend(skills);
        self
    }

    /// Build the dependency graph.
    ///
    /// This pre-computes the adjacency list and validates all edges.
    #[must_use = "builder pattern returns the constructed graph"]
    pub fn build(self) -> Result<DependencyGraph, ResolveError> {
        let mut name_index: HashMap<String, usize> = HashMap::new();
        let mut build_warnings: Vec<String> = Vec::new();

        // Build name index
        for (idx, skill) in self.skills.iter().enumerate() {
            use std::collections::hash_map::Entry;

            // Index by name
            let name_key = skill.name.clone();
            match name_index.entry(name_key.clone()) {
                Entry::Occupied(_) => {
                    build_warnings.push(format!(
                        "Duplicate skill name '{name_key}' detected; keeping first occurrence"
                    ));
                }
                Entry::Vacant(e) => {
                    e.insert(idx);
                }
            }
            // Also index by source:name
            let scoped_key = format!("{}:{}", skill.source.label(), skill.name);
            match name_index.entry(scoped_key.clone()) {
                Entry::Occupied(_) => {
                    build_warnings.push(format!(
                        "Duplicate skill key '{scoped_key}' detected; keeping first occurrence"
                    ));
                }
                Entry::Vacant(e) => {
                    e.insert(idx);
                }
            }
        }

        // Build adjacency list
        let mut edges: Vec<Vec<Edge>> = vec![Vec::new(); self.skills.len()];

        for (idx, skill) in self.skills.iter().enumerate() {
            if let Some(ref fm) = skill.frontmatter {
                let deps = fm
                    .normalized_dependencies()
                    .map_err(|e| ResolveError::ParseError {
                        skill: skill.name.clone(),
                        message: e,
                    })?;

                for dep in deps {
                    // Look up target
                    let lookup_key = match &dep.source {
                        Some(src) => format!("{}:{}", src, dep.name),
                        None => dep.name.clone(),
                    };

                    if let Some(&target_idx) = name_index.get(&lookup_key) {
                        edges[idx].push(Edge {
                            target: target_idx,
                            version_req: dep.version_req,
                            optional: dep.optional,
                        });
                    } else if !dep.optional || self.options.strict_optional {
                        return Err(ResolveError::NotFound {
                            name: dep.name,
                            required_by: skill.name.clone(),
                        });
                    } else {
                        // Optional dependency not found - add warning
                        build_warnings.push(format!(
                            "Skipped optional dependency '{}' for '{}' - not found in workspace",
                            dep.name, skill.name
                        ));
                    }
                }
            }
        }

        Ok(DependencyGraph {
            skills: self.skills,
            name_index,
            edges,
            cache: RwLock::new(HashMap::new()),
            options: self.options,
            build_warnings,
        })
    }
}

// ============================================================================
// Trait-based Registry (Original API - kept for flexibility)
// ============================================================================

/// Trait for looking up skills by name.
///
/// Implement this trait to provide skill lookup for resolution.
pub trait SkillRegistry {
    /// Look up a skill by name, optionally filtering by source.
    fn lookup(&self, name: &str, source: Option<&str>) -> Option<SkillInfo>;

    /// List all available skill names.
    fn list_skills(&self) -> Vec<String>;
}

/// Dependency resolver using trait-based registry.
///
/// Use this when you need dynamic skill lookup or can't pre-compute the graph.
/// For better performance with static skill sets, use `DependencyGraph` instead.
pub struct DependencyResolver<'a, R: SkillRegistry> {
    registry: &'a R,
    options: ResolveOptions,
}

impl<'a, R: SkillRegistry> DependencyResolver<'a, R> {
    /// Create a new resolver with the given registry and options.
    pub fn new(registry: &'a R, options: ResolveOptions) -> Self {
        Self { registry, options }
    }

    /// Resolve dependencies for a skill by name.
    #[must_use = "resolution result contains important dependency information"]
    pub fn resolve(&self, skill_name: &str) -> Result<ResolutionResult, ResolveError> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut in_stack: HashSet<String> = HashSet::new();
        let mut stack_order: Vec<String> = Vec::new();
        let mut resolved: Vec<ResolvedDependency> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        self.visit(
            skill_name,
            None,
            false,
            0,
            "root",
            &mut visited,
            &mut in_stack,
            &mut stack_order,
            &mut resolved,
            &mut warnings,
        )?;

        Ok(ResolutionResult {
            resolved,
            warnings,
            success: true,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        skill_name: &str,
        source_constraint: Option<&str>,
        optional: bool,
        depth: usize,
        required_by: &str,
        visited: &mut HashSet<String>,
        in_stack: &mut HashSet<String>,
        stack_order: &mut Vec<String>,
        resolved: &mut Vec<ResolvedDependency>,
        warnings: &mut Vec<String>,
    ) -> Result<(), ResolveError> {
        if depth > self.options.max_depth {
            return Err(ResolveError::MaxDepthExceeded(self.options.max_depth));
        }

        let key = match source_constraint {
            Some(src) => format!("{}:{}", src, skill_name),
            None => skill_name.to_string(),
        };

        if in_stack.contains(&key) {
            let cycle_start = stack_order.iter().position(|s| s == &key).unwrap_or(0);
            let cycle: Vec<_> = stack_order[cycle_start..]
                .iter()
                .chain(std::iter::once(&key))
                .cloned()
                .collect();
            return Err(ResolveError::CircularDependency {
                chain: cycle.join(" -> "),
            });
        }

        if visited.contains(&key) {
            return Ok(());
        }

        let info = match self.registry.lookup(skill_name, source_constraint) {
            Some(i) => i,
            None => {
                if optional && !self.options.strict_optional {
                    warnings.push(format!(
                        "Skipped optional dependency '{}' (not found)",
                        skill_name
                    ));
                    return Ok(());
                }
                return Err(ResolveError::NotFound {
                    name: skill_name.to_string(),
                    required_by: required_by.to_string(),
                });
            }
        };

        in_stack.insert(key.clone());
        stack_order.push(key.clone());

        if let Some(ref fm) = info.frontmatter {
            let deps = fm
                .normalized_dependencies()
                .map_err(|e| ResolveError::ParseError {
                    skill: skill_name.to_string(),
                    message: e,
                })?;

            for dep in deps {
                // Version check
                if !self.options.ignore_versions {
                    if let Some(req) = &dep.version_req {
                        if let Some(dep_info) =
                            self.registry.lookup(&dep.name, dep.source.as_deref())
                        {
                            if let Some(actual) = &dep_info.version {
                                if !req.matches(actual) {
                                    return Err(ResolveError::VersionMismatch {
                                        name: dep.name.clone(),
                                        required: req.to_string(),
                                        found: actual.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }

                self.visit(
                    &dep.name,
                    dep.source.as_deref(),
                    dep.optional,
                    depth + 1,
                    skill_name,
                    visited,
                    in_stack,
                    stack_order,
                    resolved,
                    warnings,
                )?;
            }
        }

        in_stack.remove(&key);
        stack_order.pop();
        visited.insert(key);

        resolved.push(ResolvedDependency {
            uri: info.uri.clone(),
            name: info.name.clone(),
            source: info.source.clone(),
            version: info.version.map(|v| v.to_string()),
            optional,
            depth,
        });

        Ok(())
    }
}

/// In-memory skill registry for testing and simple use cases.
#[derive(Debug, Default)]
pub struct InMemoryRegistry {
    skills: HashMap<String, SkillInfo>,
}

impl InMemoryRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a skill to the registry.
    pub fn add(&mut self, info: SkillInfo) {
        self.skills.insert(info.name.clone(), info.clone());
        self.skills
            .insert(format!("{}:{}", info.source.label(), info.name), info);
    }
}

impl SkillRegistry for InMemoryRegistry {
    fn lookup(&self, name: &str, source: Option<&str>) -> Option<SkillInfo> {
        match source {
            Some(src) => self.skills.get(&format!("{}:{}", src, name)).cloned(),
            None => self.skills.get(name).cloned(),
        }
    }

    fn list_skills(&self) -> Vec<String> {
        self.skills
            .values()
            .map(|s| s.name.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_validate::frontmatter::DeclaredDependency;

    fn make_skill(name: &str, deps: Vec<DeclaredDependency>) -> SkillInfo {
        SkillInfo {
            name: name.to_string(),
            source: SkillSource::Extra(0),
            uri: format!("skill://test/{}", name),
            version: Some(semver::Version::new(1, 0, 0)),
            frontmatter: Some(SkillFrontmatter {
                name: Some(name.to_string()),
                depends: deps,
                ..Default::default()
            }),
        }
    }

    fn make_versioned_skill(
        name: &str,
        version: (u64, u64, u64),
        deps: Vec<DeclaredDependency>,
    ) -> SkillInfo {
        SkillInfo {
            name: name.to_string(),
            source: SkillSource::Extra(0),
            uri: format!("skill://test/{}", name),
            version: Some(semver::Version::new(version.0, version.1, version.2)),
            frontmatter: Some(SkillFrontmatter {
                name: Some(name.to_string()),
                depends: deps,
                ..Default::default()
            }),
        }
    }

    // ========== DependencyResolver (trait-based) tests ==========

    #[test]
    fn test_resolver_no_dependencies() {
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill("standalone", vec![]));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("standalone").unwrap();

        assert!(result.success);
        assert_eq!(result.resolved.len(), 1);
        assert_eq!(result.resolved[0].name, "standalone");
    }

    #[test]
    fn test_resolver_simple_chain() {
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill("base", vec![]));
        registry.add(make_skill(
            "middle",
            vec![DeclaredDependency::Simple("base".to_string())],
        ));
        registry.add(make_skill(
            "top",
            vec![DeclaredDependency::Simple("middle".to_string())],
        ));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("top").unwrap();

        assert_eq!(result.resolved.len(), 3);
        assert_eq!(result.resolved[0].name, "base");
        assert_eq!(result.resolved[1].name, "middle");
        assert_eq!(result.resolved[2].name, "top");
    }

    #[test]
    fn test_resolver_circular_dependency() {
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill(
            "a",
            vec![DeclaredDependency::Simple("b".to_string())],
        ));
        registry.add(make_skill(
            "b",
            vec![DeclaredDependency::Simple("a".to_string())],
        ));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("a");

        assert!(matches!(
            result,
            Err(ResolveError::CircularDependency { .. })
        ));
    }

    #[test]
    fn test_resolver_version_mismatch() {
        let mut registry = InMemoryRegistry::new();
        registry.add(make_versioned_skill("dep", (1, 0, 0), vec![]));
        registry.add(make_skill(
            "parent",
            vec![DeclaredDependency::Simple("dep@^2.0".to_string())],
        ));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("parent");

        assert!(matches!(result, Err(ResolveError::VersionMismatch { .. })));
    }

    // ========== DependencyGraph (pre-computed) tests ==========

    #[test]
    fn test_graph_no_dependencies() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("standalone", vec![]))
            .build()
            .unwrap();

        let result = graph.resolve("standalone").unwrap();

        assert!(result.success);
        assert_eq!(result.resolved.len(), 1);
        assert_eq!(result.resolved[0].name, "standalone");
    }

    #[test]
    fn test_graph_simple_chain() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("base", vec![]))
            .add_skill(make_skill(
                "middle",
                vec![DeclaredDependency::Simple("base".to_string())],
            ))
            .add_skill(make_skill(
                "top",
                vec![DeclaredDependency::Simple("middle".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("top").unwrap();

        assert_eq!(result.resolved.len(), 3);
        assert_eq!(result.resolved[0].name, "base");
        assert_eq!(result.resolved[1].name, "middle");
        assert_eq!(result.resolved[2].name, "top");
    }

    #[test]
    fn test_graph_diamond_dependency() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("d", vec![]))
            .add_skill(make_skill(
                "b",
                vec![DeclaredDependency::Simple("d".to_string())],
            ))
            .add_skill(make_skill(
                "c",
                vec![DeclaredDependency::Simple("d".to_string())],
            ))
            .add_skill(make_skill(
                "a",
                vec![
                    DeclaredDependency::Simple("b".to_string()),
                    DeclaredDependency::Simple("c".to_string()),
                ],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("a").unwrap();

        // D should only appear once
        let names: Vec<_> = result.resolved.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names.iter().filter(|&&n| n == "d").count(), 1);

        // D should come before B and C
        let d_pos = names.iter().position(|&n| n == "d").unwrap();
        let b_pos = names.iter().position(|&n| n == "b").unwrap();
        let c_pos = names.iter().position(|&n| n == "c").unwrap();
        assert!(d_pos < b_pos);
        assert!(d_pos < c_pos);
    }

    #[test]
    fn test_graph_circular_dependency() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill(
                "a",
                vec![DeclaredDependency::Simple("b".to_string())],
            ))
            .add_skill(make_skill(
                "b",
                vec![DeclaredDependency::Simple("a".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("a");
        assert!(matches!(
            result,
            Err(ResolveError::CircularDependency { .. })
        ));
    }

    #[test]
    fn test_graph_longer_cycle() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill(
                "a",
                vec![DeclaredDependency::Simple("b".to_string())],
            ))
            .add_skill(make_skill(
                "b",
                vec![DeclaredDependency::Simple("c".to_string())],
            ))
            .add_skill(make_skill(
                "c",
                vec![DeclaredDependency::Simple("a".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("a");
        assert!(matches!(
            result,
            Err(ResolveError::CircularDependency { .. })
        ));
    }

    #[test]
    fn test_graph_version_mismatch() {
        let graph = DependencyGraph::builder()
            .add_skill(make_versioned_skill("dep", (1, 0, 0), vec![]))
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dep@^2.0".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent");
        assert!(matches!(result, Err(ResolveError::VersionMismatch { .. })));
    }

    #[test]
    fn test_graph_version_satisfied() {
        let graph = DependencyGraph::builder()
            .add_skill(make_versioned_skill("dep", (2, 5, 0), vec![]))
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dep@^2.0".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        assert_eq!(result.resolved.len(), 2);
    }

    #[test]
    fn test_graph_caching() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("base", vec![]))
            .add_skill(make_skill(
                "top",
                vec![DeclaredDependency::Simple("base".to_string())],
            ))
            .build()
            .unwrap();

        // First call computes
        let (cached_before, _) = graph.cache_stats();
        assert_eq!(cached_before, 0);

        let _ = graph.resolve("top").unwrap();

        // Second call uses cache
        let (cached_after, _) = graph.cache_stats();
        assert_eq!(cached_after, 1);

        // Result should be identical
        let result1 = graph.resolve("top").unwrap();
        let result2 = graph.resolve("top").unwrap();
        assert_eq!(result1.resolved.len(), result2.resolved.len());
    }

    #[test]
    fn test_graph_source_pinning() {
        let mut codex_skill = make_skill("shared", vec![]);
        codex_skill.source = SkillSource::Codex;
        codex_skill.uri = "skill://codex/shared".to_string();

        let mut claude_skill = make_versioned_skill("shared", (2, 0, 0), vec![]);
        claude_skill.source = SkillSource::Claude;
        claude_skill.uri = "skill://claude/shared".to_string();

        let graph = DependencyGraph::builder()
            .add_skill(codex_skill)
            .add_skill(claude_skill)
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("codex:shared".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        let shared = result.resolved.iter().find(|r| r.name == "shared").unwrap();
        assert_eq!(shared.source, SkillSource::Codex);
    }

    #[test]
    fn test_graph_max_depth() {
        let mut builder = DependencyGraph::builder().with_options(ResolveOptions {
            max_depth: 5,
            ..Default::default()
        });

        for i in 0..10 {
            let deps = if i == 0 {
                vec![]
            } else {
                vec![DeclaredDependency::Simple(format!("skill-{}", i - 1))]
            };
            builder = builder.add_skill(make_skill(&format!("skill-{}", i), deps));
        }

        let graph = builder.build().unwrap();
        let result = graph.resolve("skill-9");

        assert!(matches!(result, Err(ResolveError::MaxDepthExceeded(5))));
    }

    #[test]
    fn test_graph_duplicate_skills_warn() {
        let mut first = make_skill("dup", vec![]);
        first.uri = "skill://first/dup".to_string();
        let mut second = make_skill("dup", vec![]);
        second.uri = "skill://second/dup".to_string();

        let graph = DependencyGraph::builder()
            .add_skill(first)
            .add_skill(second)
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dup".to_string())],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Duplicate skill name 'dup'")));

        let dup = result.resolved.iter().find(|r| r.name == "dup").unwrap();
        assert_eq!(dup.uri, "skill://first/dup");
    }

    #[test]
    fn test_graph_missing_at_build() {
        let result = DependencyGraph::builder()
            .add_skill(make_skill(
                "orphan",
                vec![DeclaredDependency::Simple("nonexistent".to_string())],
            ))
            .build();

        assert!(matches!(result, Err(ResolveError::NotFound { .. })));
    }

    #[test]
    fn test_graph_optional_missing_ok() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Structured {
                    name: "missing".to_string(),
                    version: None,
                    source: None,
                    optional: true,
                }],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        assert_eq!(result.resolved.len(), 1);

        // Verify warning is present for skipped optional dependency
        assert!(
            !result.warnings.is_empty(),
            "Expected warning for skipped optional dependency"
        );
        assert!(
            result.warnings[0].contains("optional"),
            "Warning should mention 'optional'"
        );
        // Check for "not found" message template, not the dependency name "missing"
        // (which would pass by coincidence since "missing" is both the dep name
        // and a word that could appear in error messages)
        assert!(
            result.warnings[0].contains("not found"),
            "Warning should indicate the dependency was not found"
        );
        // Also verify the actual dependency name appears
        assert!(
            result.warnings[0].contains("'missing'"),
            "Warning should mention the dependency name in quotes"
        );
    }

    #[test]
    fn test_graph_optional_flag_preserved() {
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("base", vec![]))
            .add_skill(make_skill(
                "optional-child",
                vec![DeclaredDependency::Simple("base".to_string())],
            ))
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Structured {
                    name: "optional-child".to_string(),
                    version: None,
                    source: None,
                    optional: true,
                }],
            ))
            .build()
            .unwrap();

        let result = graph.resolve("parent").unwrap();
        let base = result.resolved.iter().find(|r| r.name == "base").unwrap();
        let child = result
            .resolved
            .iter()
            .find(|r| r.name == "optional-child")
            .unwrap();
        let parent = result.resolved.iter().find(|r| r.name == "parent").unwrap();

        assert!(base.optional);
        assert!(child.optional);
        assert!(!parent.optional);
    }

    // ========== Edge case tests ==========

    #[test]
    fn test_graph_len_and_is_empty() {
        // Given an empty graph
        let empty_graph = DependencyGraph::builder().build().unwrap();

        // Then len should be 0 and is_empty should be true
        assert_eq!(empty_graph.len(), 0);
        assert!(empty_graph.is_empty());

        // Given a graph with skills
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("a", vec![]))
            .add_skill(make_skill("b", vec![]))
            .build()
            .unwrap();

        // Then len should be 2 and is_empty should be false
        assert_eq!(graph.len(), 2);
        assert!(!graph.is_empty());
    }

    #[test]
    fn test_graph_get() {
        // Given a graph with a skill
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("my-skill", vec![]))
            .build()
            .unwrap();

        // When looking up by name
        let found = graph.get("my-skill");

        // Then it should find the skill
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "my-skill");

        // When looking up by source:name (Extra(0).label() = "extra0")
        let found_qualified = graph.get("extra0:my-skill");
        assert!(found_qualified.is_some());

        // When looking up non-existent
        let not_found = graph.get("nonexistent");
        assert!(not_found.is_none());
    }

    #[test]
    fn test_graph_clear_cache() {
        // Given a graph with cached resolution
        let graph = DependencyGraph::builder()
            .add_skill(make_skill("skill", vec![]))
            .build()
            .unwrap();

        let _ = graph.resolve("skill").unwrap();
        let (cached_before, _) = graph.cache_stats();
        assert_eq!(cached_before, 1);

        // When clearing cache
        graph.clear_cache();

        // Then cache should be empty
        let (cached_after, _) = graph.cache_stats();
        assert_eq!(cached_after, 0);
    }

    #[test]
    fn test_graph_add_skills_batch() {
        // Given multiple skills to add
        let skills = vec![
            make_skill("skill-1", vec![]),
            make_skill("skill-2", vec![]),
            make_skill("skill-3", vec![]),
        ];

        // When using add_skills batch method
        let graph = DependencyGraph::builder()
            .add_skills(skills)
            .build()
            .unwrap();

        // Then all skills should be present
        assert_eq!(graph.len(), 3);
        assert!(graph.get("skill-1").is_some());
        assert!(graph.get("skill-2").is_some());
        assert!(graph.get("skill-3").is_some());
    }

    #[test]
    fn test_registry_list_skills() {
        // Given a registry with skills
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill("alpha", vec![]));
        registry.add(make_skill("beta", vec![]));

        // When listing skills
        let skills = registry.list_skills();

        // Then all unique skill names should be returned
        assert!(skills.contains(&"alpha".to_string()));
        assert!(skills.contains(&"beta".to_string()));
    }

    #[test]
    fn test_graph_self_cycle() {
        // Given a skill that depends on itself
        let graph = DependencyGraph::builder()
            .add_skill(make_skill(
                "narcissist",
                vec![DeclaredDependency::Simple("narcissist".to_string())],
            ))
            .build()
            .unwrap();

        // When resolving
        let result = graph.resolve("narcissist");

        // Then it should detect the self-cycle
        assert!(matches!(
            result,
            Err(ResolveError::CircularDependency { .. })
        ));
        if let Err(ResolveError::CircularDependency { chain }) = result {
            assert!(chain.contains("narcissist"));
        }
    }

    #[test]
    fn test_resolver_optional_skipped() {
        // Given a registry with parent but missing optional dependency
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill(
            "parent",
            vec![DeclaredDependency::Structured {
                name: "optional-missing".to_string(),
                version: None,
                source: None,
                optional: true,
            }],
        ));

        // When resolving with default options (strict_optional = false)
        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("parent").unwrap();

        // Then resolution succeeds with warning
        assert!(result.success);
        assert_eq!(result.resolved.len(), 1);
        assert!(!result.warnings.is_empty());
        assert!(result.warnings[0].contains("optional"));
    }

    #[test]
    fn test_graph_prerelease_version() {
        // Given skills with prerelease versions
        let mut dep = make_versioned_skill("dep", (1, 0, 0), vec![]);
        dep.version = Some(semver::Version::parse("1.0.0-beta.1").unwrap());

        let graph = DependencyGraph::builder()
            .add_skill(dep)
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dep@^1.0.0-beta".to_string())],
            ))
            .build()
            .unwrap();

        // When resolving
        let result = graph.resolve("parent");

        // Then prerelease constraint should match prerelease version
        assert!(result.is_ok());
    }

    #[test]
    fn test_graph_ignore_versions_option() {
        // Given a version mismatch scenario
        let graph = DependencyGraph::builder()
            .with_options(ResolveOptions {
                ignore_versions: true,
                ..Default::default()
            })
            .add_skill(make_versioned_skill("dep", (1, 0, 0), vec![]))
            .add_skill(make_skill(
                "parent",
                vec![DeclaredDependency::Simple("dep@^2.0".to_string())],
            ))
            .build()
            .unwrap();

        // When resolving with ignore_versions enabled
        let result = graph.resolve("parent");

        // Then version mismatch should be ignored
        assert!(result.is_ok());
        assert_eq!(result.unwrap().resolved.len(), 2);
    }

    #[test]
    fn test_graph_not_found_error() {
        // Given an empty graph
        let graph = DependencyGraph::builder().build().unwrap();

        // When resolving non-existent skill
        let result = graph.resolve("ghost");

        // Then it should return SkillNotInGraph error
        assert!(matches!(result, Err(ResolveError::SkillNotInGraph(_))));
    }

    #[test]
    fn test_resolver_depth_tracking() {
        // Given a chain of dependencies
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill("level-0", vec![]));
        registry.add(make_skill(
            "level-1",
            vec![DeclaredDependency::Simple("level-0".to_string())],
        ));
        registry.add(make_skill(
            "level-2",
            vec![DeclaredDependency::Simple("level-1".to_string())],
        ));

        let resolver = DependencyResolver::new(&registry, ResolveOptions::default());
        let result = resolver.resolve("level-2").unwrap();

        // Then depths should be correctly tracked
        assert_eq!(result.resolved.len(), 3);
        assert_eq!(result.resolved[0].depth, 2); // level-0 at depth 2
        assert_eq!(result.resolved[1].depth, 1); // level-1 at depth 1
        assert_eq!(result.resolved[2].depth, 0); // level-2 at depth 0 (root)
    }

    #[test]
    fn test_resolver_strict_optional() {
        // Given a parent with missing optional dependency
        let mut registry = InMemoryRegistry::new();
        registry.add(make_skill(
            "parent",
            vec![DeclaredDependency::Structured {
                name: "missing-optional".to_string(),
                version: None,
                source: None,
                optional: true,
            }],
        ));

        // When resolving with strict_optional enabled
        let resolver = DependencyResolver::new(
            &registry,
            ResolveOptions {
                strict_optional: true,
                ..Default::default()
            },
        );
        let result = resolver.resolve("parent");

        // Then it should fail even though dependency is optional
        assert!(matches!(result, Err(ResolveError::NotFound { .. })));
    }
}
