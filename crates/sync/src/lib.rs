//! Cross-agent configuration sync for skrills.
//!
//! Syncs commands, MCP servers, preferences, and skills between
//! Claude Code and Codex using a pluggable adapter architecture.
//!
//! Includes validation to ensure skills are compatible with their
//! target CLI before syncing.
//!
//! # Examples
//!
//! ```
//! use skrills_sync::{
//!     AgentAdapter, Command, FieldSupport, McpServer, Preferences, Result, SyncOrchestrator,
//!     SyncParams, WriteReport,
//! };
//! use std::collections::HashMap;
//! use std::path::PathBuf;
//!
//! #[derive(Default)]
//! struct MemoryAdapter {
//!     name: &'static str,
//! }
//!
//! impl AgentAdapter for MemoryAdapter {
//!     fn name(&self) -> &str {
//!         self.name
//!     }
//!
//!     fn config_root(&self) -> PathBuf {
//!         PathBuf::new()
//!     }
//!
//!     fn supported_fields(&self) -> FieldSupport {
//!         FieldSupport {
//!             commands: true,
//!             mcp_servers: true,
//!             preferences: true,
//!             skills: true,
//!         }
//!     }
//!
//!     fn read_commands(&self, _include_marketplace: bool) -> Result<Vec<Command>> {
//!         Ok(Vec::new())
//!     }
//!
//!     fn read_mcp_servers(&self) -> Result<HashMap<String, McpServer>> {
//!         Ok(HashMap::new())
//!     }
//!
//!     fn read_preferences(&self) -> Result<Preferences> {
//!         Ok(Preferences::default())
//!     }
//!
//!     fn read_skills(&self) -> Result<Vec<Command>> {
//!         Ok(Vec::new())
//!     }
//!
//!     fn write_commands(&self, commands: &[Command]) -> Result<WriteReport> {
//!         Ok(WriteReport {
//!             written: commands.len(),
//!             ..WriteReport::default()
//!         })
//!     }
//!
//!     fn write_mcp_servers(
//!         &self,
//!         servers: &HashMap<String, McpServer>,
//!     ) -> Result<WriteReport> {
//!         Ok(WriteReport {
//!             written: servers.len(),
//!             ..WriteReport::default()
//!         })
//!     }
//!
//!     fn write_preferences(&self, _prefs: &Preferences) -> Result<WriteReport> {
//!         Ok(WriteReport::default())
//!     }
//!
//!     fn write_skills(&self, skills: &[Command]) -> Result<WriteReport> {
//!         Ok(WriteReport {
//!             written: skills.len(),
//!             ..WriteReport::default()
//!         })
//!     }
//! }
//!
//! let orchestrator =
//!     SyncOrchestrator::new(MemoryAdapter { name: "source" }, MemoryAdapter { name: "target" });
//! let report = orchestrator
//!     .sync(&SyncParams {
//!         dry_run: true,
//!         ..Default::default()
//!     })
//!     .unwrap();
//! assert!(report.success);
//! ```

#![deny(unsafe_code)]

pub type Error = anyhow::Error;
pub type Result<T> = std::result::Result<T, Error>;

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
