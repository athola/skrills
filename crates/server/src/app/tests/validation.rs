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

// -------------------------------------------------------------------------
// Review fixes: SA-4, SA-7, SA-19, SA-20, SA-31
// -------------------------------------------------------------------------

fn args(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().cloned().unwrap()
}

fn write_skill_file(root: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("SKILL.md");
    std::fs::write(&path, body).unwrap();
    path
}

fn service_for(roots: Vec<skrills_discovery::SkillRoot>) -> SkillService {
    SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap()
}

/// SA-4: the same skill in the Claude and Codex trees is compared, even
/// though the cached list keeps only one copy per name.
#[test]
fn skill_diff_compares_copies_from_each_root() {
    use skrills_discovery::{SkillRoot, SkillSource};
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _cache = crate::test_support::set_env_var(
        "SKRILLS_CACHE_PATH",
        Some(temp.path().join("cache.json").to_str().unwrap()),
    );
    let claude = temp.path().join("claude");
    let codex = temp.path().join("codex");
    let fm = "---\nname: demo\ndescription: demo skill\n---\n";
    write_skill_file(&claude, "demo", &format!("{fm}line one\n"));
    write_skill_file(&codex, "demo", &format!("{fm}line two\n"));
    let service = service_for(vec![
        SkillRoot {
            root: claude,
            source: SkillSource::Claude,
        },
        SkillRoot {
            root: codex,
            source: SkillSource::Codex,
        },
    ]);

    let result = service
        .skill_diff_tool(args(json!({"name": "demo"})))
        .unwrap();
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["locations"].as_array().unwrap().len(), 2);
    let comparison = &structured["comparisons"][0];
    assert_eq!(comparison["comparison"], "claude_vs_codex");
    assert_eq!(comparison["identical"], false);
    let diff = comparison["diff"].as_str().unwrap();
    assert!(
        diff.contains("-line one") && diff.contains("+line two"),
        "{diff}"
    );
}

/// SA-19: inserting one line at the top is one added line, not a rewrite
/// of the whole file, and the hunk header counts the real lines.
#[test]
fn unified_diff_aligns_lines_after_an_insertion() {
    let a: String = (0..200).map(|i| format!("line {i}\n")).collect();
    let b = format!("inserted\n{a}");
    let diff = super::super::tools::unified_diff(&a, &b, "a", "b", 3);
    let added = diff
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"));
    let removed = diff
        .lines()
        .filter(|l| l.starts_with('-') && !l.starts_with("---"));
    assert_eq!(added.count(), 1, "{diff}");
    assert_eq!(removed.count(), 0, "{diff}");
    assert!(diff.contains("@@ -1,3 +1,4 @@\n+inserted\n"), "{diff}");

    // Two distant edits give two hunks with matching counts.
    let mut lines: Vec<String> = (0..50).map(|i| format!("l{i}")).collect();
    let a2 = lines.join("\n");
    lines[5] = "changed".into();
    lines.remove(40);
    let b2 = lines.join("\n");
    let diff2 = super::super::tools::unified_diff(&a2, &b2, "a", "b", 1);
    assert!(
        diff2.contains("@@ -5,3 +5,3 @@\n l4\n-l5\n+changed\n l6\n"),
        "{diff2}"
    );
    assert!(
        diff2.contains("@@ -40,3 +40,2 @@\n l39\n-l40\n l41\n"),
        "{diff2}"
    );
    assert_eq!(super::super::tools::unified_diff(&a2, &a2, "a", "b", 3), "");
}

/// SA-7: an unknown peer is refused before anything is written.
#[test]
fn copilot_and_cursor_sync_tools_reject_unknown_peers() {
    let service = service_for(Vec::new());
    let cases = [
        service.sync_from_copilot_tool(args(json!({"to": "cursor", "dry_run": true}))),
        service.sync_to_copilot_tool(args(json!({"from": "cluade", "dry_run": true}))),
        service.sync_from_cursor_tool(args(json!({"to": "gemini", "dry_run": true}))),
        service.sync_to_cursor_tool(args(json!({"from": "cursor", "dry_run": true}))),
    ];
    for result in cases {
        let err = result.expect_err("unknown peer must be rejected");
        assert!(err.to_string().contains("expected one of"), "{err}");
    }
}

/// SA-20: autofix keeps a backup of user skills and leaves marketplace and
/// plugin-cache copies alone.
#[test]
fn validate_autofix_backs_up_and_skips_third_party_sources() {
    use skrills_discovery::{SkillRoot, SkillSource};
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _cache = crate::test_support::set_env_var(
        "SKRILLS_CACHE_PATH",
        Some(temp.path().join("cache.json").to_str().unwrap()),
    );
    let user = write_skill_file(&temp.path().join("user"), "mine", "no frontmatter");
    let market = write_skill_file(&temp.path().join("market"), "theirs", "no frontmatter");
    let service = service_for(vec![
        SkillRoot {
            root: temp.path().join("user"),
            source: SkillSource::Claude,
        },
        SkillRoot {
            root: temp.path().join("market"),
            source: SkillSource::Marketplace,
        },
    ]);

    service
        .validate_skills_tool(args(json!({"target": "codex", "autofix": true})))
        .unwrap();

    assert!(std::fs::read_to_string(&user).unwrap().starts_with("---"));
    assert_eq!(
        std::fs::read_to_string(user.with_extension("md.bak")).unwrap(),
        "no frontmatter",
        "a backup of the original must be kept"
    );
    assert_eq!(std::fs::read_to_string(&market).unwrap(), "no frontmatter");
}

/// SA-31: with errors_only the summary still counts every skill validated.
#[test]
fn validate_errors_only_summary_counts_all_skills() {
    use skrills_discovery::{SkillRoot, SkillSource};
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _cache = crate::test_support::set_env_var(
        "SKRILLS_CACHE_PATH",
        Some(temp.path().join("cache.json").to_str().unwrap()),
    );
    let root = temp.path().join("skills");
    write_skill_file(
        &root,
        "good",
        "---\nname: good\ndescription: A well formed skill for testing\n---\n# Good\n",
    );
    write_skill_file(&root, "bad", "no frontmatter");
    let service = service_for(vec![SkillRoot {
        root,
        source: SkillSource::Claude,
    }]);

    let result = service
        .validate_skills_tool(args(json!({"target": "codex", "errors_only": true})))
        .unwrap();
    let structured = result.structured_content.unwrap();
    assert_eq!(structured["validated"], 2);
    assert_eq!(structured["codex_valid"], 1);
    assert_eq!(structured["total"], 1, "only the failing skill is listed");
    let text = format!("{:?}", result.content);
    assert!(
        text.contains("Validated 2 skills") && text.contains("1 Codex-valid"),
        "{text}"
    );
}
