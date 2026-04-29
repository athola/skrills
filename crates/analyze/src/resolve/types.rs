//! Public types used by the dependency resolver.
//!
//! Errors, options, results, and the `SkillInfo` description carried
//! through the registry and graph layers.

use serde::{Deserialize, Serialize};
use skrills_discovery::SkillSource;
use skrills_validate::frontmatter::SkillFrontmatter;
use thiserror::Error;

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
