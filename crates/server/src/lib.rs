//! Public entry point for the `skrills` server crate.
//!
//! Core logic for the `skrills` server, organized into modules:
//!
//! - `app`: Core application entry point and MCP server.
//! - `sync`: Skill synchronization management.
//!
//! The command-line interface, `doctor` and the TUI live in the `skrills` crate.

#![deny(unsafe_code)]

mod agents_doc;
#[cfg_attr(test, allow(dead_code))]
pub mod app;
pub(crate) mod cache;
pub mod config;
mod handler;
pub mod mcp_gateway;
mod mcp_result;
pub(crate) mod metrics_types;
pub mod setup;
mod skill_trace;
#[cfg(test)]
mod test_support;
pub mod tool_schemas;

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
/// Tracing and logging configuration.
pub mod trace;
