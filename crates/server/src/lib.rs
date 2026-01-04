//! Public entry point for the `skrills` server crate.
//!
//! Core logic for the `skrills` server, organized into modules:
//!
//! - `app`: Core application entry point and MCP server.
//! - `doctor`: Configuration diagnostics.
//! - `sync`: Skill synchronization management.
//! - `tui`: Interactive terminal UI.

#![deny(unsafe_code)]

#[cfg_attr(test, allow(dead_code))]
pub mod app;
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

/// Skills manifest for caching and quick loading.
pub mod manifest;

/// HTTP transport for remote MCP access.
#[cfg(feature = "http-transport")]
pub mod http_transport;

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
