//! Trait-based dependency resolver and an in-memory registry.
//!
//! Use `DependencyResolver` when skill lookup must be dynamic (HTTP fetches,
//! filesystem polling, plugin discovery). For static skill sets, build a
//! `super::graph::DependencyGraph` instead, it caches and avoids string
//! hashing during traversal.

use super::types::{ResolutionResult, ResolveError, ResolveOptions, ResolvedDependency, SkillInfo};
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
        let mut walk = Walk::default();
        self.visit(&mut walk, skill_name, None, false, 0, "root")?;

        Ok(ResolutionResult {
            resolved: walk.resolved,
            warnings: walk.warnings,
            success: true,
        })
    }

    fn visit(
        &self,
        walk: &mut Walk,
        skill_name: &str,
        source_constraint: Option<&str>,
        optional: bool,
        depth: usize,
        required_by: &str,
    ) -> Result<(), ResolveError> {
        if depth > self.options.max_depth {
            return Err(ResolveError::MaxDepthExceeded(self.options.max_depth));
        }

        let key = match source_constraint {
            Some(src) => format!("{}:{}", src, skill_name),
            None => skill_name.to_string(),
        };

        if walk.in_stack.contains(&key) {
            let cycle_start = walk.stack_order.iter().position(|s| s == &key).unwrap_or(0);
            let cycle: Vec<_> = walk.stack_order[cycle_start..]
                .iter()
                .chain(std::iter::once(&key))
                .cloned()
                .collect();
            return Err(ResolveError::CircularDependency {
                chain: cycle.join(" -> "),
            });
        }

        if walk.visited.contains(&key) {
            return Ok(());
        }

        let info = match self.registry.lookup(skill_name, source_constraint) {
            Some(i) => i,
            None => {
                if optional && !self.options.strict_optional {
                    walk.warnings.push(format!(
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

        walk.in_stack.insert(key.clone());
        walk.stack_order.push(key.clone());

        if let Some(ref fm) = info.frontmatter {
            let deps = fm
                .normalized_dependencies()
                .map_err(|e| ResolveError::ParseError {
                    skill: skill_name.to_string(),
                    message: e,
                })?;

            for dep in deps {
                if !self.options.ignore_versions {
                    if let Some(req) = &dep.version_req {
                        if let Some(dep_info) =
                            self.registry.lookup(&dep.name, dep.source.as_deref())
                        {
                            check_version(
                                &dep.name,
                                req,
                                dep_info.version.as_ref(),
                                &mut walk.warnings,
                            )?;
                        }
                    }
                }

                // Optional propagates down: anything reached only through
                // an optional edge is itself optional (same as the graph).
                self.visit(
                    walk,
                    &dep.name,
                    dep.source.as_deref(),
                    optional || dep.optional,
                    depth + 1,
                    skill_name,
                )?;
            }
        }

        walk.in_stack.remove(&key);
        walk.stack_order.pop();
        walk.visited.insert(key);

        walk.resolved.push(ResolvedDependency {
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

/// Traversal state for one `DependencyResolver::resolve` call.
#[derive(Default)]
struct Walk {
    visited: HashSet<String>,
    in_stack: HashSet<String>,
    stack_order: Vec<String>,
    resolved: Vec<ResolvedDependency>,
    warnings: Vec<String>,
}

/// Check `req` against the dependency's declared version, shared by both
/// resolvers. A requirement on an unversioned dependency cannot be checked,
/// so it is reported as a warning instead of passing silently.
pub(super) fn check_version(
    name: &str,
    req: &semver::VersionReq,
    actual: Option<&semver::Version>,
    warnings: &mut Vec<String>,
) -> Result<(), ResolveError> {
    match actual {
        Some(actual) if !req.matches(actual) => Err(ResolveError::VersionMismatch {
            name: name.to_string(),
            required: req.to_string(),
            found: actual.to_string(),
        }),
        Some(_) => Ok(()),
        None => {
            warnings.push(format!(
                "Cannot check version requirement {req} for '{name}': it declares no version"
            ));
            Ok(())
        }
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
    ///
    /// The first skill added under a name (or `source:name`) wins, matching
    /// `GraphBuilder::build`, so both resolvers pick the same skill.
    pub fn add(&mut self, info: SkillInfo) {
        let scoped = format!("{}:{}", info.source.label(), info.name);
        self.skills
            .entry(info.name.clone())
            .or_insert_with(|| info.clone());
        self.skills.entry(scoped).or_insert(info);
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
