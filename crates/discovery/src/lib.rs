//! Core functionality for discovering skills and managing skill metadata.
//!
//! This crate provides mechanisms for:
//! - Scanning directories for skill files.
//! - Extracting metadata from skills.
//! - Managing skill sources and priority.
//! - Calculating prompt similarity using trigrams.
//!
//! # Examples
//!
//! ```
//! use skrills_discovery::{discover_skills, SkillRoot, SkillSource};
//! use tempfile::tempdir;
//!
//! let temp = tempdir().unwrap();
//! let skill_dir = temp.path().join("alpha");
//! std::fs::create_dir_all(&skill_dir).unwrap();
//! std::fs::write(skill_dir.join("SKILL.md"), "# Alpha").unwrap();
//!
//! let roots = vec![SkillRoot {
//!     root: temp.path().to_path_buf(),
//!     source: SkillSource::Codex,
//! }];
//!
//! let skills = discover_skills(&roots, None).unwrap();
//! assert_eq!(skills.len(), 1);
//! ```

#![deny(unsafe_code)]
#![warn(missing_docs)]

/// Error type for discovery operations.
pub type Error = anyhow::Error;
/// Result type for discovery operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Skill discovery and scanning functionality.
pub mod scanner;
/// Types for skill metadata and sources.
pub mod types;

pub use scanner::{
    default_priority, default_roots, discover_agents, discover_skills, extra_skill_roots,
    extract_refs_from_agents, hash_file, load_priority_override, priority_labels,
    priority_labels_and_rank_map, priority_with_override, DiscoveryConfig,
};
pub use types::{
    parse_agent_config, parse_source_key, AgentConfig, AgentMeta, Diagnostics, DuplicateInfo,
    SkillMeta, SkillRoot, SkillSource,
};
