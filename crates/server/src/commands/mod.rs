//! CLI command handlers for the skrills application.

mod agent;
mod analyze;
mod serve;
mod setup;
mod sync;
mod validate;

pub(crate) use agent::handle_agent_command;
pub(crate) use analyze::handle_analyze_command;
pub(crate) use serve::handle_serve_command;
pub(crate) use setup::handle_setup_command;
pub(crate) use sync::{handle_mirror_command, handle_sync_agents_command, handle_sync_command};
pub(crate) use validate::handle_validate_command;
