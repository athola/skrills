//! Public entry point for the `skrills` server crate.
//!
//! Core logic for the `skrills` server, organized into modules:
//!
//! - `app`: Core application entry point and MCP server.
//! - `sync`: Skill synchronization management.
//!
//! The command-line interface, `doctor` and the TUI live in the `skrills` crate.

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

/// Skill discovery mechanism.
pub mod discovery;
/// Server runtime.
pub mod runtime;
/// Tracing and logging configuration.
pub mod trace;
