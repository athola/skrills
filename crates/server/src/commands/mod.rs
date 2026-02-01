//! CLI command handlers for the skrills application.

mod agent;
mod analyze;
mod cert;
mod diff;
mod intelligence;
mod metrics;
mod recommend;
mod resolve;
mod serve;
mod setup;
mod skill;
mod sync;
mod validate;

pub(crate) use agent::handle_agent_command;
pub(crate) use analyze::handle_analyze_command;
pub(crate) use cert::{
    get_cert_status_summary, handle_cert_install_command, handle_cert_renew_command,
    handle_cert_status_command,
};
pub(crate) use diff::handle_skill_diff_command;
pub(crate) use intelligence::{
    handle_analyze_project_context_command, handle_create_skill_command,
    handle_export_analytics_command, handle_import_analytics_command,
    handle_recommend_skills_smart_command, handle_search_skills_command,
    handle_search_skills_github_command, handle_suggest_new_skills_command,
};
pub(crate) use metrics::handle_metrics_command;
pub(crate) use recommend::handle_recommend_command;
pub(crate) use resolve::handle_resolve_dependencies_command;
pub(crate) use serve::handle_serve_command;
pub(crate) use setup::handle_setup_command;
pub(crate) use skill::{
    handle_pre_commit_validate_command, handle_skill_catalog_command,
    handle_skill_deprecate_command, handle_skill_import_command, handle_skill_profile_command,
    handle_skill_rollback_command, handle_skill_score_command, handle_skill_usage_report_command,
    handle_sync_pull_command,
};
pub(crate) use sync::{handle_mirror_command, handle_sync_agents_command, handle_sync_command};
pub(crate) use validate::handle_validate_command;
