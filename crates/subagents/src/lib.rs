//! Subagent MCP integration for the skrills server.
//!
//! Exposes tools to list and run subagents across backends (e.g., Codex Responses, Claude Code).
//! Integrated into `skrills-server` via the `subagents` feature flag.

#![deny(unsafe_code)]

pub mod backend;
mod cli_detection;
pub mod registry;
pub mod service;
pub mod settings;
pub mod store;
pub mod tool_schemas;

pub use registry::{AgentRegistry, CachedAgent};
pub use service::SubagentService;
pub use store::{
    BackendKind, RunEvent, RunId, RunRecord, RunRequest, RunState, RunStatus, RunStore,
};
