//! Subagent MCP integration for the skrills server.
//!
//! This crate will expose tools to list and run subagents across multiple backends
//! (e.g., Codex Responses and Claude Code). It is wired into `skrills-server`
//! behind the `subagents` feature flag.

pub mod backend;
pub mod service;
pub mod store;

pub use service::SubagentService;
pub use store::{
    BackendKind, RunEvent, RunId, RunRecord, RunRequest, RunState, RunStatus, RunStore,
};
