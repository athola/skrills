use crate::app::SkillService;
use crate::cli::{CreateSkillMethod, OutputFormat};
use crate::discovery::merge_extra_dirs;
use anyhow::Result;
use rmcp::model::CallToolResult;
use serde_json::{json, Map as JsonMap, Value};
use skrills_state::{cache_ttl, load_manifest_settings};
use std::path::PathBuf;

fn build_service(skill_dirs: Vec<PathBuf>) -> Result<SkillService> {
    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let ttl = cache_ttl(&load_manifest_settings);
    SkillService::new_with_ttl(extra_dirs, ttl)
}

fn tool_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| content.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn print_tool_result(result: CallToolResult, format: OutputFormat) -> Result<()> {
    if format.is_json() {
        if let Some(value) = result.structured_content {
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            println!("{}", serde_json::to_string_pretty(&json!({}))?);
        }
        return Ok(());
    }

    let text = tool_text(&result);
    if !text.is_empty() {
        println!("{}", text);
    }
    Ok(())
}

/// Handle the `recommend-skills-smart` command.
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_recommend_skills_smart_command(
    uri: Option<String>,
    prompt: Option<String>,
    project_dir: Option<PathBuf>,
    limit: usize,
    include_usage: bool,
    include_context: bool,
    format: OutputFormat,
    skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    let service = build_service(skill_dirs)?;
    let mut args: JsonMap<String, Value> = JsonMap::new();

    if let Some(value) = uri {
        args.insert("uri".into(), Value::String(value));
    }
    if let Some(value) = prompt {
        args.insert("prompt".into(), Value::String(value));
    }
    if let Some(dir) = project_dir {
        args.insert(
            "project_dir".into(),
            Value::String(dir.display().to_string()),
        );
    }

    args.insert("limit".into(), json!(limit));
    args.insert("include_usage".into(), json!(include_usage));
    args.insert("include_context".into(), json!(include_context));

    let result = service.recommend_skills_smart_tool(args)?;
    print_tool_result(result, format)
}

/// Handle the `analyze-project-context` command.
pub(crate) fn handle_analyze_project_context_command(
    project_dir: Option<PathBuf>,
    include_git: bool,
    commit_limit: usize,
    format: OutputFormat,
) -> Result<()> {
    let service = build_service(Vec::new())?;
    let mut args: JsonMap<String, Value> = JsonMap::new();

    if let Some(dir) = project_dir {
        args.insert(
            "project_dir".into(),
            Value::String(dir.display().to_string()),
        );
    }
    args.insert("include_git".into(), json!(include_git));
    args.insert("commit_limit".into(), json!(commit_limit));

    let result = service.analyze_project_context_tool(args)?;
    print_tool_result(result, format)
}

/// Handle the `suggest-new-skills` command.
pub(crate) fn handle_suggest_new_skills_command(
    project_dir: Option<PathBuf>,
    focus_areas: Vec<String>,
    format: OutputFormat,
    skill_dirs: Vec<PathBuf>,
) -> Result<()> {
    let service = build_service(skill_dirs)?;
    let mut args: JsonMap<String, Value> = JsonMap::new();

    if let Some(dir) = project_dir {
        args.insert(
            "project_dir".into(),
            Value::String(dir.display().to_string()),
        );
    }
    if !focus_areas.is_empty() {
        args.insert(
            "focus_areas".into(),
            Value::Array(focus_areas.into_iter().map(Value::String).collect()),
        );
    }

    let result = service.suggest_new_skills_tool(args)?;
    print_tool_result(result, format)
}

/// Handle the `create-skill` command.
pub(crate) fn handle_create_skill_command(
    name: String,
    description: String,
    method: CreateSkillMethod,
    target_dir: Option<PathBuf>,
    project_dir: Option<PathBuf>,
    dry_run: bool,
    format: OutputFormat,
) -> Result<()> {
    let service = build_service(Vec::new())?;
    let mut args: JsonMap<String, Value> = JsonMap::new();

    args.insert("name".into(), Value::String(name));
    args.insert("description".into(), Value::String(description));
    args.insert("method".into(), Value::String(method.to_string()));
    args.insert("dry_run".into(), json!(dry_run));

    if let Some(dir) = target_dir {
        args.insert(
            "target_dir".into(),
            Value::String(dir.display().to_string()),
        );
    }
    if let Some(dir) = project_dir {
        args.insert(
            "project_dir".into(),
            Value::String(dir.display().to_string()),
        );
    }

    let result = service.create_skill_tool_sync(args)?;
    print_tool_result(result, format)
}

/// Handle the `search-skills-github` command.
pub(crate) fn handle_search_skills_github_command(
    query: String,
    limit: usize,
    format: OutputFormat,
) -> Result<()> {
    let service = build_service(Vec::new())?;
    let mut args: JsonMap<String, Value> = JsonMap::new();

    args.insert("query".into(), Value::String(query));
    args.insert("limit".into(), json!(limit));

    let result = service.search_skills_github_tool_sync(args)?;
    print_tool_result(result, format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.previous {
                std::env::set_var(self.key, v);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = std::env::var(key).ok();
        if let Some(v) = value {
            std::env::set_var(key, v);
        } else {
            std::env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    fn create_skill(dir: &std::path::Path, name: &str, content: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).expect("create skill dir");
        fs::write(skill_dir.join("SKILL.md"), content).expect("write skill");
    }

    fn skill_with_deps(name: &str, deps: &[&str]) -> String {
        let mut content = format!(
            r#"---
name: {}
description: Test skill with dependencies
---
# {}

A test skill.
"#,
            name, name
        );
        for dep in deps {
            content.push_str(&format!(
                "\nSee [{}](skill://skrills/codex/{}) for more.\n",
                dep, dep
            ));
        }
        content
    }

    #[test]
    fn test_handle_recommend_skills_smart_command() {
        let _guard = crate::test_support::env_guard();
        let home_dir = tempdir().unwrap();
        let _home = set_env_var("HOME", Some(home_dir.path().to_str().unwrap()));

        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();

        create_skill(
            &skill_dir,
            "skill-a",
            &skill_with_deps("skill-a", &["skill-b"]),
        );
        create_skill(
            &skill_dir,
            "skill-b",
            &skill_with_deps("skill-b", &["skill-c"]),
        );
        create_skill(&skill_dir, "skill-c", &skill_with_deps("skill-c", &[]));

        let result = handle_recommend_skills_smart_command(
            Some("skill://skrills/codex/skill-a".into()),
            None,
            None,
            5,
            false,
            false,
            OutputFormat::Json,
            vec![skill_dir],
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_suggest_new_skills_command() {
        let _guard = crate::test_support::env_guard();
        let home_dir = tempdir().unwrap();
        let _home = set_env_var("HOME", Some(home_dir.path().to_str().unwrap()));

        let project_dir = tempdir().unwrap();
        fs::write(
            project_dir.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\n",
        )
        .unwrap();
        let src_dir = project_dir.path().join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("lib.rs"), "pub fn demo() {}\n").unwrap();

        let skills_root = tempdir().unwrap();
        let skill_dir = skills_root.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();
        create_skill(
            &skill_dir,
            "existing-skill",
            &skill_with_deps("existing-skill", &[]),
        );

        let result = handle_suggest_new_skills_command(
            Some(project_dir.path().to_path_buf()),
            vec!["testing".to_string()],
            OutputFormat::Json,
            vec![skill_dir],
        );

        assert!(result.is_ok());
    }
}
