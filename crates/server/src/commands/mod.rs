//! CLI command handlers for the skrills application.

mod agent;
mod analyze;
mod intelligence;
mod metrics;
mod recommend;
mod resolve;
mod serve;
mod setup;
mod sync;
mod validate;

pub(crate) use agent::handle_agent_command;
pub(crate) use analyze::handle_analyze_command;
pub(crate) use intelligence::{
    handle_analyze_project_context_command, handle_create_skill_command,
    handle_export_analytics_command, handle_import_analytics_command,
    handle_recommend_skills_smart_command, handle_search_skills_github_command,
    handle_suggest_new_skills_command,
};
pub(crate) use metrics::handle_metrics_command;
pub(crate) use recommend::handle_recommend_command;
pub(crate) use resolve::handle_resolve_dependencies_command;
pub(crate) use serve::handle_serve_command;
pub(crate) use setup::handle_setup_command;
pub(crate) use sync::{handle_mirror_command, handle_sync_agents_command, handle_sync_command};
pub(crate) use validate::handle_validate_command;
