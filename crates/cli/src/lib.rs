//! Command-line surface for skrills.
//!
//! Holds the clap definitions, the per-subcommand handlers and the dispatcher
//! that routes between them. `skrills-server` keeps the MCP server, the HTTP
//! transport and the skill engine those handlers call into.

#![deny(unsafe_code)]

pub mod cli;
pub mod cold_window_cli;
pub mod commands;
pub mod dispatcher;
pub mod doctor;
pub mod tui;

pub use dispatcher::run;
