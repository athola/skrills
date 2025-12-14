//! Cross-agent configuration sync for skrills.
//!
//! Syncs commands, MCP servers, preferences, and skills between
//! Claude Code and Codex using a pluggable adapter architecture.
//!
//! Includes validation to ensure skills are compatible with their
//! target CLI before syncing.

pub mod adapters;
pub mod common;
pub mod orchestrator;
pub mod report;
pub mod validation;

pub use adapters::{AgentAdapter, ClaudeAdapter, CodexAdapter, FieldSupport};
pub use common::{Command, CommonConfig, McpServer, Preferences, SyncMeta};
pub use orchestrator::{parse_direction, SyncDirection, SyncOrchestrator, SyncParams};
pub use report::{SkipReason, SyncReport, WriteReport};
pub use validation::{
    apply_autofix_to_skill, skill_is_codex_compatible, validate_skill_for_sync,
    validate_skills_for_sync, SkillValidationResult, SyncValidationOptions, ValidationReport,
};
