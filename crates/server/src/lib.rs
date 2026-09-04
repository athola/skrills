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
pub mod cache;
pub mod config;
pub mod handler;
pub mod mcp_gateway;
mod mcp_result;
pub mod metrics_types;
pub mod setup;
pub mod skill_trace;
pub mod sync;
#[cfg(test)]
mod test_support;
pub mod tool_schemas;

/// Skills manifest for caching and quick loading.
pub mod manifest;

/// HTTP transport for remote MCP access.
#[cfg(feature = "http-transport")]
pub mod http_transport;

/// REST API endpoints for visualization dashboard.
#[cfg(feature = "http-transport")]
pub mod api;

/// Leptos-based browser UI for the dashboard.
#[cfg(feature = "http-transport")]
pub mod ui;

/// Auto-generated TLS certificate support.
#[cfg(feature = "http-transport")]
pub mod tls_auto;

/// Command-line interface for the server.
/// Skill discovery mechanism.
pub mod discovery;
/// Server runtime.
pub mod runtime;
/// Signal handling for graceful shutdown.
/// Tracing and logging configuration.
pub mod trace;
