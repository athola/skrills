//! Validation tests - Skill validation, autofix, and dependency checking

use super::super::*;
use serde_json::json;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn validate_skills_tool_autofix_adds_frontmatter() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().expect("create temp directory");
    let skill_dir = temp.path().join("skills");
    std::fs::create_dir_all(&skill_dir).expect("create skill directory");
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(&skill_path, "A skill without frontmatter").expect("write test skill file");

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard =
        crate::test_support::set_env_var("HOME", Some(temp.path().to_str().expect("temp path")));

    let service = SkillService::new_with_ttl(vec![skill_dir.clone()], Duration::from_secs(1))
        .expect("create skill service");
    let result = service
        .validate_skills_tool(
            json!({"target": "codex", "autofix": true})
                .as_object()
                .cloned()
                .expect("create json args"),
        )
        .expect("validate skills");

    let content = std::fs::read_to_string(&skill_path).expect("read skill file");
    assert!(
        content.starts_with("---"),
        "autofix should add frontmatter to skill files"
    );
    let structured = result.structured_content.expect("structured content");
    assert_eq!(
        structured.get("autofixed").and_then(|v| v.as_u64()),
        Some(1)
    );
}

#[test]
fn create_skill_rejects_path_like_names() {
    let service =
        SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).expect("create service");
    let args = json!({
        "name": "../escape",
        "description": "invalid",
        "method": "github",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .expect("create json args");

    let err = service
        .create_skill_tool_sync(args)
        .expect_err("should reject path-like name");
    assert!(err.to_string().contains("Invalid name"));
}

#[test]
fn validate_skills_tool_dependency_validation() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let skill_dir = temp.path().join("skills");
    std::fs::create_dir_all(&skill_dir).unwrap();

    // Create a skill with missing local dependencies
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(
        &skill_path,
        r#"---
name: test-skill
description: A test skill with dependencies
---
# Test Skill

This skill references:
- [Missing module](modules/helper.md)
- [Missing reference](references/guide.md)
- [Existing file](../other.md)
"#,
    )
    .unwrap();

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service =
        SkillService::new_with_ttl(vec![skill_dir.clone()], Duration::from_secs(1)).unwrap();

    // Validate without dependency checking
    let result_no_deps = service
        .validate_skills_tool(
            json!({"target": "both", "check_dependencies": false})
                .as_object()
                .cloned()
                .unwrap(),
        )
        .unwrap();

    let structured_no_deps = result_no_deps.structured_content.unwrap();
    let results_no_deps = structured_no_deps
        .get("results")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(results_no_deps.len(), 1);
    assert!(results_no_deps[0].get("dependency_issues").is_none());

    // Validate with dependency checking
    let result_with_deps = service
        .validate_skills_tool(
            json!({"target": "both", "check_dependencies": true})
                .as_object()
                .cloned()
                .unwrap(),
        )
        .unwrap();

    let structured_with_deps = result_with_deps.structured_content.unwrap();
    let results_with_deps = structured_with_deps
        .get("results")
        .unwrap()
        .as_array()
        .unwrap();
    assert_eq!(results_with_deps.len(), 1);

    let skill_result = &results_with_deps[0];
    let dep_issues = skill_result
        .get("dependency_issues")
        .unwrap()
        .as_array()
        .unwrap();
    let missing_count = skill_result.get("missing_count").unwrap().as_u64().unwrap();

    // Should find missing modules and references
    assert!(
        missing_count >= 2,
        "Expected at least 2 missing dependencies, found {}",
        missing_count
    );

    // Check that dependency issues have the right structure
    let has_missing_module = dep_issues
        .iter()
        .any(|i| i.get("type").unwrap().as_str().unwrap() == "missing_module");
    let has_missing_reference = dep_issues
        .iter()
        .any(|i| i.get("type").unwrap().as_str().unwrap() == "missing_reference");

    assert!(
        has_missing_module,
        "Expected to find missing_module issue type"
    );
    assert!(
        has_missing_reference,
        "Expected to find missing_reference issue type"
    );

    // Verify the summary includes dependency issues
    assert_eq!(
        structured_with_deps.get("check_dependencies").unwrap(),
        &json!(true)
    );
    let total_dep_issues = structured_with_deps
        .get("total_dependency_issues")
        .unwrap()
        .as_u64()
        .unwrap();
    assert!(
        total_dep_issues >= 2,
        "Expected at least 2 total dependency issues"
    );
}
