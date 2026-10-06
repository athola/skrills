//! Pre-computed dependency graph and its builder.
//!
//! `DependencyGraph` is the fast path: build once, resolve many. The graph
//! caches resolution results behind an `RwLock` so repeated lookups are O(1).
//! `GraphBuilder` validates edges at build time so resolution itself can avoid
//! string lookups.

use super::resolver::check_version;
use super::types::{ResolutionResult, ResolveError, ResolveOptions, ResolvedDependency, SkillInfo};
use parking_lot::RwLock;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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

/// Traversal state for one `DependencyGraph::compute_resolution` call.
#[derive(Default)]
struct IndexWalk {
    visited: HashSet<usize>,
    in_stack: HashSet<usize>,
    stack_path: Vec<usize>,
    result: CachedResolution,
}

/// Cached resolution result.
#[derive(Debug, Clone, Default)]
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
        let mut walk = IndexWalk::default();
        self.visit_index(&mut walk, root_idx, false, 0)?;
        Ok(walk.result)
    }

    fn visit_index(
        &self,
        walk: &mut IndexWalk,
        idx: usize,
        optional: bool,
        depth: usize,
    ) -> Result<(), ResolveError> {
        // Depth limit
        if depth > self.options.max_depth {
            return Err(ResolveError::MaxDepthExceeded(self.options.max_depth));
        }

        // Cycle detection
        if walk.in_stack.contains(&idx) {
            let cycle_start = walk.stack_path.iter().position(|&i| i == idx).unwrap_or(0);
            let cycle_names: Vec<_> = walk.stack_path[cycle_start..]
                .iter()
                .chain(std::iter::once(&idx))
                .map(|&i| self.skills[i].name.as_str())
                .collect();
            return Err(ResolveError::CircularDependency {
                chain: cycle_names.join(" -> "),
            });
        }

        // Already resolved
        if walk.visited.contains(&idx) {
            return Ok(());
        }

        // Mark in-progress
        walk.in_stack.insert(idx);
        walk.stack_path.push(idx);

        // Visit dependencies
        for edge in &self.edges[idx] {
            // Handle missing (edges only point to valid indices, but check anyway)
            debug_assert!(
                edge.target < self.skills.len(),
                "edge target {} out of bounds (max index: {})",
                edge.target,
                self.skills.len().saturating_sub(1)
            );
            if edge.target >= self.skills.len() {
                if edge.optional && !self.options.strict_optional {
                    walk.result
                        .warnings
                        .push("Skipped optional dependency (invalid index)".to_string());
                    continue;
                }
                return Err(ResolveError::NotFound {
                    name: format!("index-{}", edge.target),
                    required_by: self.skills[idx].name.clone(),
                });
            }

            // Version check
            if !self.options.ignore_versions {
                if let Some(req) = &edge.version_req {
                    let target = &self.skills[edge.target];
                    check_version(
                        &target.name,
                        req,
                        target.version.as_ref(),
                        &mut walk.result.warnings,
                    )?;
                }
            }

            let child_optional = optional || edge.optional;
            self.visit_index(walk, edge.target, child_optional, depth + 1)?;
        }

        // Done processing
        walk.in_stack.remove(&idx);
        walk.stack_path.pop();
        walk.visited.insert(idx);

        // Post-order: add after dependencies
        walk.result.order.push(idx);
        walk.result.depths.push(depth);
        walk.result.optionals.push(optional);

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
