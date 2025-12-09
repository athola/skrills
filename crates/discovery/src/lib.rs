//! Core functionality for discovering skills and managing skill metadata.
//!
//! This crate provides mechanisms for:
//! - Scanning directories for skill files.
//! - Extracting metadata from skills.
//! - Managing skill sources and priority.
//! - Calculating prompt similarity using trigrams.

pub mod scanner;
pub mod types;

pub use scanner::{
    default_priority, default_roots, discover_agents, discover_skills, extra_skill_roots,
    extract_refs_from_agents, hash_file, load_priority_override, priority_labels,
    priority_labels_and_rank_map, priority_with_override, DiscoveryConfig,
};
pub use types::{
    parse_source_key, AgentMeta, Diagnostics, DuplicateInfo, SkillMeta, SkillRoot, SkillSource,
};
