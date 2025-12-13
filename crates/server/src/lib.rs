//! Public entry point for the `skrills` server crate.
//!
//! This crate provides the core logic for the `skrills` server, organized into
//! modules for maintainability:
//!
//! - `app`: Core application entry point and MCP server.
//! - `autoload`: Handles skill autoloading and rendering.
//! - `doctor`: Provides configuration diagnostics.
//! - `emit`: Manages hook emission for shell integration.
//! - `sync`: Manages skill synchronization.
//! - `tui`: Implements the interactive terminal UI.

mod app;
mod autoload;
mod commands;
mod doctor;
mod emit;
mod setup;
mod sync;
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
