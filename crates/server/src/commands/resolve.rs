use crate::app::SkillService;
use crate::cli::DependencyDirection;
use crate::discovery::merge_extra_dirs;
use anyhow::Result;
use serde_json::json;
use skrills_state::{cache_ttl, load_manifest_settings};
use std::path::PathBuf;

/// Handle the `resolve-dependencies` command.
pub(crate) fn handle_resolve_dependencies_command(
    uri: String,
    skill_dirs: Vec<PathBuf>,
    direction: DependencyDirection,
    transitive: bool,
    format: String,
) -> Result<()> {
    let extra_dirs = merge_extra_dirs(&skill_dirs);
    let ttl = cache_ttl(&load_manifest_settings);
    let service = SkillService::new_with_ttl(extra_dirs, ttl)?;

    let mut cache = service.cache.lock();
    cache.ensure_fresh()?;
    if cache.skill_by_uri(&uri).is_err() {
        anyhow::bail!("Skill not found: {}", uri);
    }

    let results = match (direction, transitive) {
        (DependencyDirection::Dependencies, true) => cache.resolve_dependencies(&uri)?,
        (DependencyDirection::Dependencies, false) => cache.get_direct_dependencies(&uri)?,
        (DependencyDirection::Dependents, true) => cache.get_transitive_dependents(&uri)?,
        (DependencyDirection::Dependents, false) => cache.get_dependents(&uri)?,
    };

    let direction_label = match (direction, transitive) {
        (DependencyDirection::Dependencies, true) => "transitive dependencies",
        (DependencyDirection::Dependencies, false) => "direct dependencies",
        (DependencyDirection::Dependents, true) => "transitive dependents",
        (DependencyDirection::Dependents, false) => "direct dependents",
    };

    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "uri": uri,
                "direction": match direction {
                    DependencyDirection::Dependencies => "dependencies",
                    DependencyDirection::Dependents => "dependents",
                },
                "transitive": transitive,
                "results": results,
                "count": results.len(),
            }))?
        );
        return Ok(());
    }

    println!("Found {} {} for {}", results.len(), direction_label, uri);
    if results.is_empty() {
        println!("No {} found.", direction_label);
    } else {
        for item in results {
            println!("  - {}", item);
        }
    }

    Ok(())
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
    fn test_handle_resolve_dependencies_command() {
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

        let result = handle_resolve_dependencies_command(
            "skill://skrills/extra0/skill-a/SKILL.md".into(),
            vec![skill_dir],
            DependencyDirection::Dependencies,
            true,
            "json".into(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_handle_resolve_dependencies_unknown_uri() {
        let _guard = crate::test_support::env_guard();
        let home_dir = tempdir().unwrap();
        let _home = set_env_var("HOME", Some(home_dir.path().to_str().unwrap()));

        let tmp = tempdir().unwrap();
        let skill_dir = tmp.path().join("skills");
        fs::create_dir_all(&skill_dir).unwrap();

        create_skill(&skill_dir, "skill-a", &skill_with_deps("skill-a", &[]));

        let result = handle_resolve_dependencies_command(
            "skill://skrills/extra0/missing-skill/SKILL.md".into(),
            vec![skill_dir],
            DependencyDirection::Dependencies,
            true,
            "json".into(),
        );

        assert!(result.is_err());
    }
}
