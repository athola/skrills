//! Public entry point for the `skrills` server crate.
//!
//! This crate provides the core logic for the `skrills` server, organized into
//! modules for maintainability:
//!
//! - `app`: Core application entry point and MCP server.
//! - `doctor`: Provides configuration diagnostics.
//! - `sync`: Manages skill synchronization.
//! - `tui`: Implements the interactive terminal UI.

#[cfg_attr(test, allow(dead_code))]
pub(crate) mod app;
pub(crate) mod cache;
mod commands;
mod doctor;
mod handler;
pub(crate) mod metrics_types;
mod setup;
mod skill_trace;
mod sync;
#[cfg(test)]
mod test_support;
mod tool_schemas;
mod tui;

/// Command-line interface for the server.
pub mod cli;
/// Skill discovery mechanism.
pub mod discovery;
/// Server runtime.
pub mod runtime;
/// Signal handling for graceful shutdown.
pub mod signals;
/// Tracing and logging configuration.
pub mod trace;

pub use app::run;
