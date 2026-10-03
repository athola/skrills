//! CLI command handlers for the skrills application.

mod agent;
mod analyze;
mod cert;
mod diff;
mod intelligence;
mod metrics;
mod multi_cli_agent;
mod recommend;
mod resolve;
mod serve;
mod setup;
mod skill;
mod skill_uri;
mod sync;
mod validate;

pub(crate) use agent::handle_agent_command;
pub(crate) use analyze::handle_analyze_command;
#[cfg(feature = "http-transport")]
pub(crate) use cert::get_cert_status_summary;
pub(crate) use cert::{
    handle_cert_install_command, handle_cert_renew_command, handle_cert_status_command,
};
pub(crate) use diff::handle_skill_diff_command;
pub(crate) use intelligence::{
    handle_analyze_project_context_command, handle_create_skill_command,
    handle_export_analytics_command, handle_import_analytics_command,
    handle_recommend_skills_smart_command, handle_search_skills_command,
    handle_search_skills_github_command, handle_suggest_new_skills_command, SmartRecommendOptions,
};
pub(crate) use metrics::handle_metrics_command;
pub(crate) use multi_cli_agent::handle_multi_cli_agent_command;
pub(crate) use multi_cli_agent::is_available as is_on_path;
pub(crate) use recommend::handle_recommend_command;
pub(crate) use resolve::handle_resolve_dependencies_command;
pub(crate) use serve::{handle_serve_command, ServeOptions};
pub(crate) use setup::{handle_setup_command, SetupOptions};
pub(crate) use skill::{
    handle_pre_commit_validate_command, handle_skill_catalog_command,
    handle_skill_deprecate_command, handle_skill_import_command, handle_skill_profile_command,
    handle_skill_rollback_command, handle_skill_score_command, handle_skill_usage_report_command,
    handle_sync_pull_command,
};
pub(crate) use sync::{
    codex_skills_feature_warning, handle_mirror_command, handle_sync_agents_command,
    handle_sync_all_command, handle_sync_command, handle_sync_status_command,
    run_sync_with_adapters, skipped_commands_note, SyncAllArgs,
};
pub(crate) use validate::handle_validate_command;

use skrills_discovery::{skill_roots_or_default, SkillRoot};
use std::path::PathBuf;

/// Skill roots for a command that takes `--skill-dir`.
///
/// The directories given on the command line and in `SKRILLS_SKILL_DIR` when
/// there are any, every default root otherwise. The help text of these
/// commands says "default: all discovered skills", and building the roots from
/// the extra directories alone found nothing when neither was set.
pub(crate) fn skill_roots_for(skill_dirs: &[PathBuf]) -> Vec<SkillRoot> {
    skill_roots_or_default(&skrills_server::discovery::merge_extra_dirs(skill_dirs))
}

#[cfg(test)]
mod tests {
    use super::skill_roots_for;
    use std::path::PathBuf;

    #[test]
    fn skill_roots_for_without_dirs_falls_back_to_the_default_roots() {
        let _g = skrills_test_utils::env_guard();
        let home = tempfile::tempdir().unwrap();
        let _home = skrills_test_utils::set_env_var("HOME", Some(home.path().to_str().unwrap()));
        let _dirs = skrills_test_utils::set_env_var("SKRILLS_SKILL_DIR", None);

        let roots = skill_roots_for(&[]);

        assert!(
            roots
                .iter()
                .any(|r| r.root == home.path().join(".claude/skills")),
            "no --skill-dir should search ~/.claude/skills, got {:?}",
            roots.iter().map(|r| &r.root).collect::<Vec<_>>()
        );
    }

    #[test]
    fn skill_roots_for_with_dirs_searches_only_those() {
        let _g = skrills_test_utils::env_guard();
        let _dirs = skrills_test_utils::set_env_var("SKRILLS_SKILL_DIR", None);
        let dir = PathBuf::from("/tmp/only-here");

        let roots = skill_roots_for(std::slice::from_ref(&dir));

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].root, dir);
    }
}
