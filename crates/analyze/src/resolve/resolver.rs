//! Trait-based dependency resolver and an in-memory registry.
//!
//! Use `DependencyResolver` when skill lookup must be dynamic (HTTP fetches,
//! filesystem polling, plugin discovery). For static skill sets, build a
//! `super::graph::DependencyGraph` instead — it caches and avoids string
//! hashing during traversal.

use super::types::{
    ResolutionResult, ResolveError, ResolveOptions, ResolvedDependency, SkillInfo,
};
use std::collections::{HashMap, HashSet};

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
