//! Tests for the app module.
//!
//! These tests verify core SkillService functionality including:
//! - Skill validation with autofix
//! - Dependency graph resolution
//! - Resource reading with dependency resolution
//! - URI query parameter parsing
//! - Sync operations

use super::*;
use serde_json::json;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn default_skill_root_prefers_claude_when_both_installed() {
    let home = PathBuf::from("/home/test");
    let path = select_default_skill_root(&home, true, true);

    assert_eq!(path, home.join(".claude/skills"));
}

#[test]
fn default_skill_root_uses_codex_when_only_codex_installed() {
    let home = PathBuf::from("/home/test");
    let path = select_default_skill_root(&home, false, true);

    assert_eq!(path, home.join(".codex/skills"));
}

#[test]
fn resolve_project_dir_prefers_explicit_path() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let path = temp.path().join("project");
    let resolved = resolve_project_dir(path.to_str(), "test");

    assert_eq!(resolved, Some(path));
}

#[test]
fn resolve_project_dir_uses_current_dir() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let original = std::env::current_dir().unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let resolved = resolve_project_dir(None, "test");

    std::env::set_current_dir(original).unwrap();
    assert_eq!(resolved, Some(temp.path().to_path_buf()));
}

#[cfg(unix)]
#[test]
fn resolve_project_dir_returns_none_when_cwd_missing() {
    let _guard = crate::test_support::env_guard();
    let original = std::env::current_dir().unwrap();
    let temp = tempdir().unwrap();
    let gone = temp.path().join("gone");
    std::fs::create_dir_all(&gone).unwrap();
    std::env::set_current_dir(&gone).unwrap();
    std::fs::remove_dir_all(&gone).unwrap();

    let resolved = resolve_project_dir(None, "test");

    std::env::set_current_dir(original).unwrap();
    assert!(resolved.is_none());
}

#[test]
fn validate_skills_tool_autofix_adds_frontmatter() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let skill_dir = temp.path().join("skills");
    std::fs::create_dir_all(&skill_dir).unwrap();
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(&skill_path, "A skill without frontmatter").unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service =
        SkillService::new_with_ttl(vec![skill_dir.clone()], Duration::from_secs(1)).unwrap();
    let result = service
        .validate_skills_tool(
            json!({"target": "codex", "autofix": true})
                .as_object()
                .cloned()
                .unwrap(),
        )
        .unwrap();

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    let content = std::fs::read_to_string(&skill_path).unwrap();
    assert!(
        content.starts_with("---"),
        "autofix should add frontmatter to skill files"
    );
    let structured = result.structured_content.unwrap();
    assert_eq!(
        structured.get("autofixed").and_then(|v| v.as_u64()),
        Some(1)
    );
}

#[test]
fn create_skill_rejects_path_like_names() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();
    let args = json!({
        "name": "../escape",
        "description": "invalid",
        "method": "github",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let err = service.create_skill_tool_sync(args).unwrap_err();
    assert!(err.to_string().contains("Invalid name"));
}

#[test]
fn test_dependency_graph_integration() {
    use skrills_discovery::SkillRoot;

    // Initialize tracing for test
    let _ = tracing_subscriber::fmt()
        .with_env_filter("skrills::deps=debug")
        .try_init();

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill A (depends on B and C)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B and C
---
# Skill A
See [skill-b](../skill-b/SKILL.md) and [skill-c](../skill-c/SKILL.md) for details.
"#,
    )
    .unwrap();

    // Create skill B (depends on D)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B depends on D
---
# Skill B
Uses [skill-d](../skill-d/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill C (depends on D)
    let skill_c_dir = skills_dir.join("skill-c");
    fs::create_dir_all(&skill_c_dir).unwrap();
    fs::write(
        skill_c_dir.join("SKILL.md"),
        r#"---
name: skill-c
description: Skill C depends on D
---
# Skill C
Also uses [skill-d](../skill-d/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill D (no dependencies)
    let skill_d_dir = skills_dir.join("skill-d");
    fs::create_dir_all(&skill_d_dir).unwrap();
    fs::write(
        skill_d_dir.join("SKILL.md"),
        r#"---
name: skill-d
description: Skill D has no dependencies
---
# Skill D
Base skill with no dependencies.
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

    // Force refresh to build the graph
    service.invalidate_cache().unwrap();
    let skills = service.current_skills_with_dups().unwrap().0;

    // Verify skills were discovered
    assert_eq!(skills.len(), 4);

    // Test dependency resolution
    let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md";
    let deps = service.resolve_dependencies(skill_a_uri).unwrap();

    // Debug output
    eprintln!("skill-a dependencies: {:?}", deps);
    eprintln!(
        "Skills discovered: {:?}",
        skills.iter().map(|s| &s.name).collect::<Vec<_>>()
    );

    // skill-a should have transitive dependencies: skill-b, skill-c, skill-d
    assert!(
        deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()),
        "Expected skill-b in deps"
    );
    assert!(
        deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()),
        "Expected skill-c in deps"
    );
    assert!(
        deps.contains(&"skill://skrills/extra0/skill-d/SKILL.md".to_string()),
        "Expected skill-d in deps"
    );

    // Test reverse dependencies
    let skill_d_uri = "skill://skrills/extra0/skill-d/SKILL.md";
    let dependents = service.get_dependents(skill_d_uri).unwrap();

    // skill-d should be used by skill-b and skill-c
    assert!(dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    assert!(dependents.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));

    // Test transitive dependents
    let trans_deps = service.get_transitive_dependents(skill_d_uri).unwrap();

    // skill-d should transitively affect skill-a, skill-b, skill-c
    assert!(trans_deps.contains(&"skill://skrills/extra0/skill-a/SKILL.md".to_string()));
    assert!(trans_deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    assert!(trans_deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));
}

#[test]
fn test_resolve_dependencies_tool() {
    use skrills_discovery::SkillRoot;

    // Initialize tracing for test
    let _ = tracing_subscriber::fmt()
        .with_env_filter("skrills::deps=debug")
        .try_init();

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill A (depends on B)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md) for details.
"#,
    )
    .unwrap();

    // Create skill B (depends on C)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B depends on C
---
# Skill B
Uses [skill-c](../skill-c/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill C (no dependencies)
    let skill_c_dir = skills_dir.join("skill-c");
    fs::create_dir_all(&skill_c_dir).unwrap();
    fs::write(
        skill_c_dir.join("SKILL.md"),
        r#"---
name: skill-c
description: Skill C has no dependencies
---
# Skill C
Base skill.
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

    // Force refresh to build the graph
    service.invalidate_cache().unwrap();

    // Test 1: Transitive dependencies for A (should get B and C)
    let deps = service
        .resolve_dependencies("skill://skrills/extra0/skill-a/SKILL.md")
        .unwrap();
    assert_eq!(deps.len(), 2);
    assert!(deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    assert!(deps.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));

    // Test 2: Direct dependencies for A (should only get B)
    let mut cache = service.cache.lock();
    let direct_deps = cache
        .get_direct_dependencies("skill://skrills/extra0/skill-a/SKILL.md")
        .unwrap();
    assert_eq!(direct_deps.len(), 1);
    assert!(direct_deps.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    drop(cache);

    // Test 3: Direct dependents of C (should only get B)
    let dependents = service
        .get_dependents("skill://skrills/extra0/skill-c/SKILL.md")
        .unwrap();
    assert_eq!(dependents.len(), 1);
    assert!(dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));

    // Test 4: Transitive dependents of C (should get A and B)
    let trans_dependents = service
        .get_transitive_dependents("skill://skrills/extra0/skill-c/SKILL.md")
        .unwrap();
    assert_eq!(trans_dependents.len(), 2);
    assert!(trans_dependents.contains(&"skill://skrills/extra0/skill-a/SKILL.md".to_string()));
    assert!(trans_dependents.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
}

#[test]
fn test_read_resource_without_resolve() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill A (depends on B)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md) for details.
"#,
    )
    .unwrap();

    // Create skill B (no dependencies)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B
---
# Skill B
Base skill.
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

    // Test reading without resolve param
    let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md";
    let result = service.read_resource_sync(skill_a_uri).unwrap();

    // Should return only the requested skill
    assert_eq!(result.contents.len(), 1);
    let content = &result.contents[0];
    if let ResourceContents::TextResourceContents {
        uri, text, meta, ..
    } = content
    {
        assert_eq!(uri, skill_a_uri);
        assert!(text.contains("Skill A"));
        assert!(text.contains("depends on B"));
        // Check metadata indicates this is the requested resource
        let meta = meta.as_ref().unwrap();
        assert_eq!(meta.get("role").and_then(|v| v.as_str()), Some("requested"));
    } else {
        panic!("Expected TextResourceContents");
    }
}

#[test]
fn test_read_resource_with_resolve_true() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill A (depends on B and C)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B and C
---
# Skill A
See [skill-b](../skill-b/SKILL.md) and [skill-c](../skill-c/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill B (depends on D)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B depends on D
---
# Skill B
Uses [skill-d](../skill-d/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill C (depends on D)
    let skill_c_dir = skills_dir.join("skill-c");
    fs::create_dir_all(&skill_c_dir).unwrap();
    fs::write(
        skill_c_dir.join("SKILL.md"),
        r#"---
name: skill-c
description: Skill C depends on D
---
# Skill C
Also uses [skill-d](../skill-d/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill D (no dependencies)
    let skill_d_dir = skills_dir.join("skill-d");
    fs::create_dir_all(&skill_d_dir).unwrap();
    fs::write(
        skill_d_dir.join("SKILL.md"),
        r#"---
name: skill-d
description: Skill D
---
# Skill D
Base skill.
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

    // Test reading with resolve=true
    let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md?resolve=true";
    let result = service.read_resource_sync(skill_a_uri).unwrap();

    // Should return requested skill + all transitive dependencies (B, C, D)
    assert_eq!(result.contents.len(), 4);

    // First item should be the requested skill
    let first = &result.contents[0];
    if let ResourceContents::TextResourceContents {
        uri, text, meta, ..
    } = first
    {
        assert_eq!(uri, "skill://skrills/extra0/skill-a/SKILL.md");
        assert!(text.contains("Skill A"));
        let meta = meta.as_ref().unwrap();
        assert_eq!(meta.get("role").and_then(|v| v.as_str()), Some("requested"));
    } else {
        panic!("Expected TextResourceContents");
    }

    // Check that dependencies are included
    let uris: Vec<String> = result
        .contents
        .iter()
        .filter_map(|c| match c {
            ResourceContents::TextResourceContents { uri, .. } => Some(uri.clone()),
            _ => None,
        })
        .collect();

    assert!(uris.contains(&"skill://skrills/extra0/skill-b/SKILL.md".to_string()));
    assert!(uris.contains(&"skill://skrills/extra0/skill-c/SKILL.md".to_string()));
    assert!(uris.contains(&"skill://skrills/extra0/skill-d/SKILL.md".to_string()));

    // Check that dependencies have correct role metadata
    for content in &result.contents[1..] {
        if let ResourceContents::TextResourceContents { meta, .. } = content {
            let meta = meta.as_ref().unwrap();
            assert_eq!(
                meta.get("role").and_then(|v| v.as_str()),
                Some("dependency")
            );
        }
    }
}

#[test]
fn test_read_resource_with_resolve_false() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill A (depends on B)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill B
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B
---
# Skill B
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();

    // Test reading with resolve=false (explicit)
    let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md?resolve=false";
    let result = service.read_resource_sync(skill_a_uri).unwrap();

    // Should return only the requested skill (same as no param)
    assert_eq!(result.contents.len(), 1);
    let content = &result.contents[0];
    if let ResourceContents::TextResourceContents {
        uri, text, meta, ..
    } = content
    {
        assert_eq!(uri, "skill://skrills/extra0/skill-a/SKILL.md");
        assert!(text.contains("Skill A"));
        let meta = meta.as_ref().unwrap();
        assert_eq!(meta.get("role").and_then(|v| v.as_str()), Some("requested"));
    } else {
        panic!("Expected TextResourceContents");
    }
}

#[test]
fn test_parse_uri_with_query() {
    // Test basic URI without query
    let (base, resolve) = parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md");
    assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
    assert!(!resolve);

    // Test with resolve=true
    let (base, resolve) =
        parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?resolve=true");
    assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
    assert!(resolve);

    // Test with resolve=false
    let (base, resolve) =
        parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?resolve=false");
    assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
    assert!(!resolve);

    // Test with resolve shorthand
    let (base, resolve) = parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?resolve");
    assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
    assert!(resolve);

    // Test with multiple params
    let (base, resolve) =
        parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?foo=bar&resolve=true");
    assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
    assert!(resolve);

    // Test with multiple params, resolve first
    let (base, resolve) =
        parse_uri_with_query("skill://skrills/extra0/skill-a/SKILL.md?resolve=true&foo=bar");
    assert_eq!(base, "skill://skrills/extra0/skill-a/SKILL.md");
    assert!(resolve);
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

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

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

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }
}

/// Tests for recommend_skills method - URI validation
/// GIVEN a SkillService with valid skills
/// WHEN recommend_skills is called with a non-existent URI
/// THEN it should return an error indicating the skill was not found
#[test]
fn test_recommend_skills_uri_not_found_returns_error() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a valid skill
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A
---
# Skill A
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Try to get recommendations for non-existent skill
    let result = service.recommend_skills("skill://skrills/extra0/nonexistent/SKILL.md", 10, false);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Skill not found"),
        "Expected 'Skill not found' error, got: {}",
        err_msg
    );
}

/// Tests for recommend_skills method - basic functionality
/// GIVEN a SkillService with skills that have dependencies
/// WHEN recommend_skills is called for a skill with dependencies
/// THEN it should return recommendations including dependencies, dependents, and siblings
#[test]
fn test_recommend_skills_returns_dependencies_and_dependents() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill A (depends on B)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill B (depends on C)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B depends on C
---
# Skill B
Uses [skill-c](../skill-c/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill C (no dependencies)
    let skill_c_dir = skills_dir.join("skill-c");
    fs::create_dir_all(&skill_c_dir).unwrap();
    fs::write(
        skill_c_dir.join("SKILL.md"),
        r#"---
name: skill-c
description: Skill C
---
# Skill C
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Get recommendations for skill-b (has both dependency and dependent)
    let result = service
        .recommend_skills("skill://skrills/extra0/skill-b/SKILL.md", 10, false)
        .unwrap();

    assert_eq!(result.source_uri, "skill://skrills/extra0/skill-b/SKILL.md");

    // Should have skill-c as dependency (base score 3.0)
    let deps: Vec<_> = result
        .recommendations
        .iter()
        .filter(|r| matches!(r.relationship, RecommendationRelationship::Dependency))
        .collect();
    assert!(!deps.is_empty(), "Expected at least one dependency");

    // Should have skill-a as dependent (base score 2.0)
    let dependents: Vec<_> = result
        .recommendations
        .iter()
        .filter(|r| matches!(r.relationship, RecommendationRelationship::Dependent))
        .collect();
    assert!(!dependents.is_empty(), "Expected at least one dependent");
}

/// Tests for recommend_skills method - quality scoring
/// GIVEN a SkillService with skills
/// WHEN recommend_skills is called with include_quality=true
/// THEN it should include quality scores in recommendations
#[test]
fn test_recommend_skills_includes_quality_scores_when_requested() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill A (depends on B)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: A high-quality skill with proper documentation
---
# Skill A

This is a well-documented skill that follows best practices.

See [skill-b](../skill-b/SKILL.md) for more.
"#,
    )
    .unwrap();

    // Create skill B
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Another well-documented skill
---
# Skill B

This skill is also well-documented.
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Get recommendations with quality scoring
    let result = service
        .recommend_skills("skill://skrills/extra0/skill-a/SKILL.md", 10, true)
        .unwrap();

    // Should have quality scores for dependencies
    for rec in &result.recommendations {
        if matches!(rec.relationship, RecommendationRelationship::Dependency) {
            assert!(
                rec.quality_score.is_some(),
                "Expected quality_score for dependency, got None"
            );
            // Score should include quality bonus (base 3.0 + quality)
            assert!(
                rec.score > 3.0,
                "Expected score > 3.0 with quality bonus, got {}",
                rec.score
            );
        }
    }
}

/// Tests for recommend_skills method - sibling detection
/// GIVEN a SkillService with skills that share common dependencies
/// WHEN recommend_skills is called for a skill
/// THEN it should include sibling skills in recommendations
#[test]
fn test_recommend_skills_finds_siblings_with_shared_dependencies() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create common dependency (skill-d)
    let skill_d_dir = skills_dir.join("skill-d");
    fs::create_dir_all(&skill_d_dir).unwrap();
    fs::write(
        skill_d_dir.join("SKILL.md"),
        r#"---
name: skill-d
description: Common dependency
---
# Skill D
"#,
    )
    .unwrap();

    // Create skill A (depends on D)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A
---
# Skill A
Uses [skill-d](../skill-d/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill B (also depends on D - making it a sibling of A)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B
---
# Skill B
Also uses [skill-d](../skill-d/SKILL.md).
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Get recommendations for skill-a
    let result = service
        .recommend_skills("skill://skrills/extra0/skill-a/SKILL.md", 10, false)
        .unwrap();

    // Should have skill-b as sibling (base score 1.0)
    let siblings: Vec<_> = result
        .recommendations
        .iter()
        .filter(|r| matches!(r.relationship, RecommendationRelationship::Sibling))
        .collect();

    assert!(
        !siblings.is_empty(),
        "Expected at least one sibling (skill-b shares dependency on skill-d)"
    );
}

#[tokio::test]
async fn sync_all_tool_syncs_skills_into_codex_skills_root() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let claude_skill = temp.path().join(".claude/skills/example-skill/SKILL.md");
    std::fs::create_dir_all(claude_skill.parent().unwrap()).unwrap();
    std::fs::write(&claude_skill, "example skill").unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(vec![], Duration::from_secs(1)).unwrap();
    let _ = service
        .sync_all_tool(
            json!({
                "from": "claude",
                "dry_run": false,
                "skip_existing_commands": true
            })
            .as_object()
            .cloned()
            .unwrap(),
        )
        .unwrap();

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    // `sync_skills_only_from_claude` preserves paths relative to ~/.claude.
    let expected = temp
        .path()
        .join(".codex/skills/skills/example-skill/SKILL.md");
    assert!(
        expected.exists(),
        "expected skill copied into ~/.codex/skills"
    );

    let unexpected = temp
        .path()
        .join(".codex/skills-mirror/skills/example-skill/SKILL.md");
    assert!(
        !unexpected.exists(),
        "sync-all should not write skills into ~/.codex/skills-mirror"
    );
}

// -------------------------------------------------------------------------
// Fuzzy Search Tests (search_skills_fuzzy_tool)
// -------------------------------------------------------------------------

/// Tests for search_skills_fuzzy_tool - basic functionality
/// GIVEN a SkillService with skills
/// WHEN search_skills_fuzzy is called with an exact match query
/// THEN it should return the matching skill with high similarity
#[test]
fn test_search_skills_fuzzy_exact_match() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a skill named "database" - note that the actual name stored includes SKILL.md
    let skill_dir = skills_dir.join("database");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: database
description: Database operations and queries
---
# Database Skill
Handles database operations.
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Search for exact match - name will be "database/SKILL.md"
    let args = json!({
        "query": "database",
        "threshold": 0.3,
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();

    // Should find the skill
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    let total = structured
        .get("total_found")
        .and_then(|v| v.as_u64())
        .unwrap();
    assert!(total >= 1, "Expected at least 1 match, got {}", total);

    let results = structured.get("results").unwrap().as_array().unwrap();
    let first = &results[0];
    // Name includes the path like "database/SKILL.md"
    let name = first.get("name").and_then(|v| v.as_str()).unwrap();
    assert!(
        name.contains("database"),
        "Expected name to contain 'database', got '{}'",
        name
    );
}

/// Tests for search_skills_fuzzy_tool - typo tolerance
/// GIVEN a SkillService with skills
/// WHEN search_skills_fuzzy is called with a typo query
/// THEN it should still find the matching skill
#[test]
fn test_search_skills_fuzzy_typo_tolerance() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a skill named "database"
    let skill_dir = skills_dir.join("database");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: database
description: Database operations
---
# Database Skill
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Search with typo "databas" (missing 'e')
    let args = json!({
        "query": "databas",
        "threshold": 0.3,
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();

    // Should still find the skill
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    let total = structured
        .get("total_found")
        .and_then(|v| v.as_u64())
        .unwrap();
    assert!(
        total >= 1,
        "Expected to find 'database' with typo 'databas'"
    );

    let results = structured.get("results").unwrap().as_array().unwrap();
    let first = &results[0];
    // Name includes the path like "database/SKILL.md"
    let name = first.get("name").and_then(|v| v.as_str()).unwrap();
    assert!(
        name.contains("database"),
        "Expected name to contain 'database', got '{}'",
        name
    );
}

/// Tests for search_skills_fuzzy_tool - no matches
/// GIVEN a SkillService with skills
/// WHEN search_skills_fuzzy is called with an unrelated query
/// THEN it should return empty results
#[test]
fn test_search_skills_fuzzy_no_matches() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a skill named "database"
    let skill_dir = skills_dir.join("database");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: database
description: Database operations
---
# Database Skill
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Search for completely unrelated term
    let args = json!({
        "query": "xyznonexistent",
        "threshold": 0.5,
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();

    // Should return empty results
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    let total = structured
        .get("total_found")
        .and_then(|v| v.as_u64())
        .unwrap();
    assert_eq!(total, 0, "Expected no matches for unrelated query");
}

/// Tests for search_skills_fuzzy_tool - threshold filtering
/// GIVEN a SkillService with skills
/// WHEN search_skills_fuzzy is called with high threshold
/// THEN only high-similarity matches should be returned
#[test]
fn test_search_skills_fuzzy_threshold_filtering() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skills with varying name similarity
    for name in ["database", "data-tools", "cache-manager"] {
        let skill_dir = skills_dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                r#"---
name: {}
description: Skill for {}
---
# {} Skill
"#,
                name, name, name
            ),
        )
        .unwrap();
    }

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Search with high threshold (0.8) - should only match "database"
    let args = json!({
        "query": "database",
        "threshold": 0.8,
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();
    let structured = result.structured_content.unwrap();
    let results = structured.get("results").unwrap().as_array().unwrap();

    // With threshold 0.8, only exact/near-exact matches should pass
    for r in results {
        let similarity = r.get("similarity").and_then(|v| v.as_f64()).unwrap();
        assert!(
            similarity >= 0.8,
            "Expected all results above threshold 0.8, got {}",
            similarity
        );
    }
}

/// Tests for search_skills_fuzzy_tool - limit parameter
/// GIVEN a SkillService with many skills
/// WHEN search_skills_fuzzy is called with a limit
/// THEN only the top N results should be returned
#[test]
fn test_search_skills_fuzzy_limit() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create multiple skills with similar names
    for i in 1..=5 {
        let name = format!("test-skill-{}", i);
        let skill_dir = skills_dir.join(&name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                r#"---
name: {}
description: Test skill number {}
---
# Test Skill {}
"#,
                name, i, i
            ),
        )
        .unwrap();
    }

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Search with limit of 2
    let args = json!({
        "query": "test",
        "threshold": 0.1,
        "limit": 2
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();
    let structured = result.structured_content.unwrap();
    let results = structured.get("results").unwrap().as_array().unwrap();

    assert!(
        results.len() <= 2,
        "Expected at most 2 results, got {}",
        results.len()
    );
}

/// Tests for search_skills_fuzzy_tool - missing query parameter
/// GIVEN a SkillService
/// WHEN search_skills_fuzzy is called without a query
/// THEN it should return an error
#[test]
fn test_search_skills_fuzzy_missing_query() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "threshold": 0.5,
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("query"),
        "Expected error about missing query, got: {}",
        err_msg
    );
}

/// Tests for search_skills_fuzzy_tool - empty query string
/// GIVEN a SkillService with skills
/// WHEN search_skills_fuzzy is called with an empty query string
/// THEN it should return zero results (not an error)
#[test]
fn test_search_skills_fuzzy_empty_query_string() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    let skill_dir = skills_dir.join("test-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\n---\n# Test\n",
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    let args = json!({
        "query": "",
        "threshold": 0.3
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    let total = structured
        .get("total_found")
        .and_then(|v| v.as_u64())
        .unwrap();
    assert_eq!(total, 0, "Empty query should return no results");
}

/// Tests for search_skills_fuzzy_tool - whitespace-only query
/// GIVEN a SkillService with skills
/// WHEN search_skills_fuzzy is called with whitespace-only query
/// THEN it should return zero results
#[test]
fn test_search_skills_fuzzy_whitespace_query() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    let skill_dir = skills_dir.join("test-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\n---\n# Test\n",
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    let args = json!({
        "query": "   ",
        "threshold": 0.3
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    let total = structured
        .get("total_found")
        .and_then(|v| v.as_u64())
        .unwrap();
    assert_eq!(total, 0, "Whitespace-only query should return no results");
}

/// Tests for search_skills_fuzzy_tool - case insensitivity
/// GIVEN a SkillService with skills
/// WHEN search_skills_fuzzy is called with different case
/// THEN it should still find matches
#[test]
fn test_search_skills_fuzzy_case_insensitive() {
    use skrills_discovery::SkillRoot;

    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill with lowercase name
    let skill_dir = skills_dir.join("database");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: database
description: Database operations
---
# Database Skill
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Search with uppercase
    let args = json!({
        "query": "DATABASE",
        "threshold": 0.5,
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();
    let structured = result.structured_content.unwrap();
    let total = structured
        .get("total_found")
        .and_then(|v| v.as_u64())
        .unwrap();

    assert!(
        total >= 1,
        "Expected case-insensitive match for DATABASE -> database"
    );
}

// -------------------------------------------------------------------------
// Tool Handler Tests (tools.rs)
// -------------------------------------------------------------------------

/// Tests for parse_trace_target helper function
/// GIVEN various target argument values
/// WHEN parse_trace_target is called
/// THEN it should return the correct TraceTarget enum
#[test]
fn test_parse_trace_target_claude_returns_claude_target() {
    let args = json!({"target": "claude"}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(format!("{:?}", target), "Claude");
}

#[test]
fn test_parse_trace_target_codex_returns_codex_target() {
    let args = json!({"target": "codex"}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(format!("{:?}", target), "Codex");
}

#[test]
fn test_parse_trace_target_both_or_invalid_returns_both_target() {
    // Test "both" explicitly
    let args = json!({"target": "both"}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(format!("{:?}", target), "Both");

    // Test missing/invalid target (defaults to both)
    let args = json!({}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(format!("{:?}", target), "Both");

    // Test random invalid value
    let args = json!({"target": "invalid"}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(format!("{:?}", target), "Both");
}

/// Tests for skill_loading_status_tool
/// GIVEN a SkillService
/// WHEN skill_loading_status_tool is called
/// THEN it should return status with structured content
#[test]
fn test_skill_loading_status_tool_returns_status() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "both"}).as_object().cloned().unwrap();
    let result = service.skill_loading_status_tool(args).unwrap();

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    // Should not error and should have structured content
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    // Should contain skill_files_found field (may be 0 in test environment)
    assert!(structured.get("skill_files_found").is_some());
    // Should contain instrumented_markers_found field
    assert!(structured.get("instrumented_markers_found").is_some());
}

/// Tests for skill_loading_status_tool with options
/// GIVEN a SkillService
/// WHEN skill_loading_status_tool is called with optional flags
/// THEN it should accept and process the options
#[test]
fn test_skill_loading_status_tool_accepts_optional_flags() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Test with various optional flags
    let args = json!({
        "target": "claude",
        "include_cache": true,
        "include_marketplace": false,
        "include_mirror": true,
        "include_agent": false
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.skill_loading_status_tool(args);

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    // Should not error when processing flags
    assert!(
        result.is_ok(),
        "skill_loading_status_tool should accept optional flags"
    );
}

/// Tests for skill_loading_selftest_tool
/// GIVEN a SkillService
/// WHEN skill_loading_selftest_tool is called
/// THEN it should return probe configuration
#[test]
fn test_skill_loading_selftest_tool_returns_probe_config() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "both", "dry_run": false})
        .as_object()
        .cloned()
        .unwrap();

    let result = service.skill_loading_selftest_tool(args).unwrap();

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();

    // Should contain probe_line and expected_response
    assert!(structured.get("probe_line").is_some());
    assert!(structured.get("expected_response").is_some());
    assert!(structured.get("target").is_some());

    // probe_line and expected_response should match format
    let probe_line = structured.get("probe_line").unwrap().as_str().unwrap();
    assert!(probe_line.starts_with("SKRILLS_PROBE:"));
}

/// Tests for skill_loading_selftest_tool with dry_run
/// GIVEN a SkillService
/// WHEN skill_loading_selftest_tool is called with dry_run=true
/// THEN it should still return valid probe config
#[test]
fn test_skill_loading_selftest_tool_dry_run_returns_config() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "claude", "dry_run": true})
        .as_object()
        .cloned()
        .unwrap();

    let result = service.skill_loading_selftest_tool(args).unwrap();

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();

    // Even with dry_run, should get valid probe config
    let probe_line = structured.get("probe_line").unwrap().as_str().unwrap();
    assert!(probe_line.starts_with("SKRILLS_PROBE:"));

    // Should contain notes array
    let notes = structured.get("notes").and_then(|v| v.as_array()).unwrap();
    assert!(
        !notes.is_empty(),
        "Expected notes array with helpful information"
    );
}

/// Tests for disable_skill_trace_tool
/// GIVEN a SkillService
/// WHEN disable_skill_trace_tool is called with dry_run
/// THEN it should return removal info without actual removal
#[test]
fn test_disable_skill_trace_tool_dry_run_returns_info() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "both", "dry_run": true})
        .as_object()
        .cloned()
        .unwrap();

    let result = service.disable_skill_trace_tool(args).unwrap();

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    assert!(!result.is_error.unwrap_or(true));

    // For dry_run, structured content should have dry_run flag
    let structured = result.structured_content.unwrap();
    assert_eq!(structured.get("dry_run").unwrap(), &json!(true));
    // removed field should indicate what would be removed
    assert!(structured.get("removed").is_some());
}

/// Tests for disable_skill_trace_tool with different targets
/// GIVEN a SkillService
/// WHEN disable_skill_trace_tool is called for different targets
/// THEN it should accept claude, codex, and both
#[test]
fn test_disable_skill_trace_tool_accepts_all_targets() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    for target in ["claude", "codex", "both"] {
        let args = json!({"target": target, "dry_run": true})
            .as_object()
            .cloned()
            .unwrap();

        let result = service.disable_skill_trace_tool(args);

        match original_home {
            Some(ref val) => std::env::set_var("HOME", val),
            None => std::env::remove_var("HOME"),
        }

        assert!(
            result.is_ok(),
            "disable_skill_trace_tool should accept target '{}'",
            target
        );
    }

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }
}

/// Tests for enable_skill_trace_tool - integration test for file operations
/// GIVEN a SkillService with a skills directory
/// WHEN enable_skill_trace_tool is called
/// THEN it should create trace skill files and instrument skill files
#[test]
fn test_enable_skill_trace_tool_creates_trace_files() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    // Create the Claude skills directory structure
    let claude_skills = temp.path().join(".claude/skills");
    fs::create_dir_all(&claude_skills).unwrap();

    // Create a sample skill to be instrumented
    let skill_dir = claude_skills.join("test-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: test-skill
description: A test skill
---
# Test Skill
This is a test skill content.
"#,
    )
    .unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service =
        SkillService::new_with_ttl(vec![claude_skills.clone()], Duration::from_secs(1)).unwrap();

    let args = json!({
        "target": "claude",
        "instrument": true,
        "backup": true,
        "dry_run": false,
        "include_mirror": false,
        "include_agent": false
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.enable_skill_trace_tool(args).unwrap();

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    // Verify the result
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();

    // Check that trace skill was installed
    let installed_trace = structured
        .get("installed_trace_skill")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(installed_trace, "Expected trace skill to be installed");

    // Check that probe skill was installed
    let installed_probe = structured
        .get("installed_probe_skill")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(installed_probe, "Expected probe skill to be installed");

    // Verify the trace skill file exists
    // Uses constant from skill_trace.rs: TRACE_SKILL_DIR = "skrills-skill-trace"
    let trace_skill_path = claude_skills.join("skrills-skill-trace/SKILL.md");
    assert!(
        trace_skill_path.exists(),
        "Expected trace skill file to be created at {:?}",
        trace_skill_path
    );

    // Verify the probe skill file exists
    // Uses constant from skill_trace.rs: PROBE_SKILL_DIR = "skrills-skill-probe"
    let probe_skill_path = claude_skills.join("skrills-skill-probe/SKILL.md");
    assert!(
        probe_skill_path.exists(),
        "Expected probe skill file to be created at {:?}",
        probe_skill_path
    );

    // Check that files were instrumented
    let instrumented_count = structured
        .get("instrumented_files")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    assert!(
        instrumented_count >= 1,
        "Expected at least 1 file to be instrumented"
    );
}

/// Tests for enable_skill_trace_tool - dry_run mode
/// GIVEN a SkillService
/// WHEN enable_skill_trace_tool is called with dry_run=true
/// THEN it should return report without creating files
#[test]
fn test_enable_skill_trace_tool_dry_run_no_file_creation() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let claude_skills = temp.path().join(".claude/skills");
    fs::create_dir_all(&claude_skills).unwrap();

    let skill_dir = claude_skills.join("test-skill");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: test-skill\n---\n# Test\n",
    )
    .unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service =
        SkillService::new_with_ttl(vec![claude_skills.clone()], Duration::from_secs(1)).unwrap();

    let args = json!({
        "target": "claude",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.enable_skill_trace_tool(args).unwrap();

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    assert!(!result.is_error.unwrap_or(true));

    // In dry_run mode, trace skill files should NOT be created
    let trace_skill_path = claude_skills.join("skill-loading-trace/SKILL.md");
    assert!(
        !trace_skill_path.exists(),
        "dry_run should NOT create trace skill file"
    );
}

/// Tests for skill_loading_status_tool - error path when home_dir fails
/// GIVEN a SkillService with HOME unset (simulated)
/// WHEN skill_loading_status_tool is called
/// THEN it should handle the error gracefully
#[cfg(unix)]
#[test]
fn test_skill_loading_status_tool_handles_missing_home() {
    let _guard = crate::test_support::env_guard();

    // Temporarily unset HOME to simulate home_dir() failing
    let original_home = std::env::var("HOME").ok();
    std::env::remove_var("HOME");

    // Also remove alternate home env vars that home_dir() might check
    let original_user_profile = std::env::var("USERPROFILE").ok();
    std::env::remove_var("USERPROFILE");

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "both"}).as_object().cloned().unwrap();
    let result = service.skill_loading_status_tool(args);

    // Restore environment
    if let Some(val) = original_home {
        std::env::set_var("HOME", val);
    }
    if let Some(val) = original_user_profile {
        std::env::set_var("USERPROFILE", val);
    }

    // The result should either be an error or return gracefully with status
    // Either is acceptable - we're testing it doesn't panic
    match result {
        Ok(r) => {
            // If it succeeds, verify it has expected fields
            if let Some(structured) = r.structured_content {
                assert!(
                    structured.get("skill_files_found").is_some()
                        || structured.get("error").is_some()
                );
            }
        }
        Err(e) => {
            // Error is acceptable - verify it's a meaningful error
            let msg = e.to_string();
            assert!(
                msg.contains("home") || msg.contains("HOME") || msg.contains("directory"),
                "Expected home directory related error, got: {}",
                msg
            );
        }
    }
}

// =========================================================================
// Intelligence Tool Integration Tests
// =========================================================================

// -------------------------------------------------------------------------
// recommend_skills_smart_tool Tests
// -------------------------------------------------------------------------

/// Tests for recommend_skills_smart_tool - basic functionality
/// GIVEN a SkillService with skills
/// WHEN recommend_skills_smart_tool is called
/// THEN it should return recommendations with structured content
#[test]
fn test_recommend_skills_smart_tool_basic() {
    use skrills_discovery::SkillRoot;

    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create test skills
    for name in ["database", "api-client", "auth-service"] {
        let skill_dir = skills_dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                r#"---
name: {}
description: Skill for {} operations
---
# {} Skill
"#,
                name, name, name
            ),
        )
        .unwrap();
    }

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    let args = json!({
        "prompt": "database",
        "limit": 5,
        "include_usage": false,
        "include_context": false
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.recommend_skills_smart_tool(args).unwrap();

    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    assert!(structured.get("total_found").is_some());
    assert!(structured.get("recommendations").is_some());
}

/// Tests for recommend_skills_smart_tool - with URI parameter
/// GIVEN a SkillService with skills that have dependencies
/// WHEN recommend_skills_smart_tool is called with a source URI
/// THEN it should return dependency-based recommendations
#[test]
fn test_recommend_skills_smart_tool_with_uri() {
    use skrills_discovery::SkillRoot;

    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create skill A (depends on B)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).unwrap();
    fs::write(
        skill_a_dir.join("SKILL.md"),
        r#"---
name: skill-a
description: Skill A depends on B
---
# Skill A
See [skill-b](../skill-b/SKILL.md).
"#,
    )
    .unwrap();

    // Create skill B
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).unwrap();
    fs::write(
        skill_b_dir.join("SKILL.md"),
        r#"---
name: skill-b
description: Skill B
---
# Skill B
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    let args = json!({
        "uri": "skill://skrills/extra0/skill-a/SKILL.md",
        "limit": 10,
        "include_usage": false,
        "include_context": false
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.recommend_skills_smart_tool(args).unwrap();

    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    let recommendations = structured
        .get("recommendations")
        .unwrap()
        .as_array()
        .unwrap();

    // Should include skill-b as a dependency
    let has_skill_b = recommendations.iter().any(|r| {
        r.get("uri")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .contains("skill-b")
    });
    assert!(has_skill_b, "Expected skill-b in recommendations");
}

/// Tests for recommend_skills_smart_tool - default parameters
/// GIVEN a SkillService
/// WHEN recommend_skills_smart_tool is called with minimal args
/// THEN it should use sensible defaults
#[test]
fn test_recommend_skills_smart_tool_defaults() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Empty args should work with defaults
    let args = json!({}).as_object().cloned().unwrap();

    let result = service.recommend_skills_smart_tool(args);

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }

    // Should not error even with no skills
    assert!(result.is_ok());
}

// -------------------------------------------------------------------------
// create_skill_tool Tests - Input Validation & Security
// -------------------------------------------------------------------------

/// Tests for create_skill_tool - path traversal prevention
/// GIVEN a SkillService
/// WHEN create_skill_tool is called with path traversal in name
/// THEN it should reject the request
#[test]
fn test_create_skill_tool_rejects_path_traversal() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Test various path traversal attempts
    let traversal_attempts = [
        "../escape",
        "..\\escape",
        "foo/../bar",
        "foo/../../etc/passwd",
        "..%2F..%2Fetc",
    ];

    for attempt in traversal_attempts {
        let args = json!({
            "name": attempt,
            "description": "malicious skill",
            "method": "github",
            "dry_run": true
        })
        .as_object()
        .cloned()
        .unwrap();

        let result = service.create_skill_tool_sync(args);
        assert!(
            result.is_err(),
            "Expected error for path traversal attempt: {}",
            attempt
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Invalid name"),
            "Expected 'Invalid name' in error for {}, got: {}",
            attempt,
            err_msg
        );
    }
}

/// Tests for create_skill_tool - hidden file prevention
/// GIVEN a SkillService
/// WHEN create_skill_tool is called with hidden file name
/// THEN it should reject the request
#[test]
fn test_create_skill_tool_rejects_hidden_files() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let hidden_names = [".hidden", ".secret-skill", "..double-dot"];

    for name in hidden_names {
        let args = json!({
            "name": name,
            "description": "hidden skill",
            "method": "github",
            "dry_run": true
        })
        .as_object()
        .cloned()
        .unwrap();

        let result = service.create_skill_tool_sync(args);
        assert!(
            result.is_err(),
            "Expected error for hidden file name: {}",
            name
        );
    }
}

/// Tests for create_skill_tool - rejects names with path separators
/// GIVEN a SkillService
/// WHEN create_skill_tool is called with path separators in name
/// THEN it should reject the request
#[test]
fn test_create_skill_tool_rejects_path_separators() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Forward slash is always a path separator on all platforms
    // Note: backslash is only a separator on Windows, so we only test forward slash
    let invalid_names = ["foo/bar", "skills/malicious"];

    for name in invalid_names {
        let args = json!({
            "name": name,
            "description": "skill with path",
            "method": "github",
            "dry_run": true
        })
        .as_object()
        .cloned()
        .unwrap();

        let result = service.create_skill_tool_sync(args);
        assert!(
            result.is_err(),
            "Expected error for name with path separator: {}",
            name
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Invalid name") || err_msg.contains("path"),
            "Expected path-related error for {}, got: {}",
            name,
            err_msg
        );
    }
}

/// Tests for create_skill_tool - requires name parameter
/// GIVEN a SkillService
/// WHEN create_skill_tool is called without name
/// THEN it should return an error
#[test]
fn test_create_skill_tool_requires_name() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "description": "a skill",
        "method": "github",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.create_skill_tool_sync(args);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("name"),
        "Expected error about missing name"
    );
}

/// Tests for create_skill_tool - requires description parameter
/// GIVEN a SkillService
/// WHEN create_skill_tool is called without description
/// THEN it should return an error
#[test]
fn test_create_skill_tool_requires_description() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "name": "valid-skill",
        "method": "github",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.create_skill_tool_sync(args);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("description"),
        "Expected error about missing description"
    );
}

/// Tests for create_skill_tool - valid name acceptance
/// GIVEN a SkillService
/// WHEN create_skill_tool is called with valid simple names
/// THEN it should accept them (in dry_run mode)
#[test]
fn test_create_skill_tool_accepts_valid_names() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let valid_names = [
        "my-skill",
        "my_skill",
        "MySkill",
        "skill123",
        "a",
        "very-long-skill-name-with-many-parts",
    ];

    for name in valid_names {
        let args = json!({
            "name": name,
            "description": "a valid skill",
            "method": "github",
            "dry_run": true
        })
        .as_object()
        .cloned()
        .unwrap();

        // Should not error on validation (may error on network, but that's ok)
        let result = service.create_skill_tool_sync(args);
        // In dry_run mode with github method, it makes a network call
        // We just verify the name validation passed
        if let Err(e) = &result {
            let err_msg = e.to_string();
            assert!(
                !err_msg.contains("Invalid name"),
                "Valid name {} was rejected: {}",
                name,
                err_msg
            );
        }
    }
}

/// Tests for create_skill_tool - invalid method handling
/// GIVEN a SkillService
/// WHEN create_skill_tool is called with invalid method
/// THEN it should return an error
#[test]
fn test_create_skill_tool_invalid_method() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "name": "valid-skill",
        "description": "a skill",
        "method": "invalid-method",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.create_skill_tool_sync(args);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Invalid creation method") || err_msg.contains("method"),
        "Expected method-related error, got: {}",
        err_msg
    );
}

// -------------------------------------------------------------------------
// search_skills_github_tool Tests
// -------------------------------------------------------------------------

/// Tests for search_skills_github_tool - requires query parameter
/// GIVEN a SkillService
/// WHEN search_skills_github_tool is called without query
/// THEN it should return an error
#[test]
fn test_search_skills_github_requires_query() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_github_tool_sync(args);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("query"),
        "Expected error about missing query"
    );
}

/// Tests for search_skills_github_tool - default limit
/// GIVEN a SkillService
/// WHEN search_skills_github_tool is called without limit
/// THEN it should use default limit of 10
#[test]
fn test_search_skills_github_default_limit() {
    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // This will make a network call, so we can only test the args are accepted
    let args = json!({
        "query": "rust async"
    })
    .as_object()
    .cloned()
    .unwrap();

    // The call may fail due to network/rate limits, but shouldn't fail on validation
    let result = service.search_skills_github_tool_sync(args);
    if let Err(e) = &result {
        let err_msg = e.to_string();
        assert!(
            !err_msg.contains("limit") && !err_msg.contains("Missing"),
            "Validation should accept missing limit, got: {}",
            err_msg
        );
    }
}

// -------------------------------------------------------------------------
// analyze_project_context_tool Tests
// -------------------------------------------------------------------------

/// Tests for analyze_project_context_tool - with explicit project_dir
/// GIVEN a SkillService and a project directory
/// WHEN analyze_project_context_tool is called with project_dir
/// THEN it should analyze the project
#[test]
fn test_analyze_project_context_with_explicit_dir() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("my-project");
    fs::create_dir_all(&project_dir).unwrap();

    // Create a minimal package.json to simulate a Node.js project
    fs::write(
        project_dir.join("package.json"),
        r#"{"name": "test-project", "version": "1.0.0"}"#,
    )
    .unwrap();

    // Create a JavaScript file
    fs::write(project_dir.join("index.js"), "console.log('hello');").unwrap();

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "project_dir": project_dir.to_str().unwrap(),
        "include_git": false,
        "max_languages": 5
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.analyze_project_context_tool(args).unwrap();

    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();

    // Should have languages detected
    assert!(structured.get("languages").is_some());
    assert!(structured.get("project_type").is_some());
}

/// Tests for analyze_project_context_tool - error when no project_dir and no cwd
/// GIVEN a SkillService
/// WHEN analyze_project_context_tool is called without project_dir and cwd is invalid
/// THEN it should return an error
#[cfg(unix)]
#[test]
fn test_analyze_project_context_requires_valid_dir() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let gone_dir = temp.path().join("gone");
    fs::create_dir_all(&gone_dir).unwrap();

    let original_cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(&gone_dir).unwrap();
    fs::remove_dir_all(&gone_dir).unwrap();

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({}).as_object().cloned().unwrap();

    let result = service.analyze_project_context_tool(args);

    std::env::set_current_dir(original_cwd).unwrap();

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("directory") || err_msg.contains("project_dir"),
        "Expected directory-related error, got: {}",
        err_msg
    );
}

/// Tests for analyze_project_context_tool - respects options
/// GIVEN a SkillService
/// WHEN analyze_project_context_tool is called with custom options
/// THEN it should respect them
#[test]
fn test_analyze_project_context_respects_options() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();

    // Create some files
    fs::write(project_dir.join("main.py"), "print('hello')").unwrap();

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "project_dir": project_dir.to_str().unwrap(),
        "include_git": false,
        "commit_limit": 10,
        "max_languages": 3
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.analyze_project_context_tool(args);
    assert!(result.is_ok());
}

// -------------------------------------------------------------------------
// suggest_new_skills_tool Tests
// -------------------------------------------------------------------------

/// Tests for suggest_new_skills_tool - identifies skill gaps
/// GIVEN a SkillService with skills and a project
/// WHEN suggest_new_skills_tool is called
/// THEN it should identify gaps between project needs and existing skills
#[test]
fn test_suggest_new_skills_identifies_gaps() {
    use skrills_discovery::SkillRoot;

    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    // Create skills directory with one skill
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    let db_skill_dir = skills_dir.join("database");
    fs::create_dir_all(&db_skill_dir).unwrap();
    fs::write(
        db_skill_dir.join("SKILL.md"),
        "---\nname: database\n---\n# Database Skill\n",
    )
    .unwrap();

    // Create project directory with Python files
    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join("main.py"), "print('hello')").unwrap();
    fs::write(project_dir.join("app.py"), "# flask app").unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    let args = json!({
        "project_dir": project_dir.to_str().unwrap(),
        "focus_areas": ["testing", "deployment"]
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.suggest_new_skills_tool(args).unwrap();

    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();

    // Should have gaps identified
    let gaps = structured.get("gaps").and_then(|v| v.as_array());
    assert!(gaps.is_some(), "Expected gaps array in response");

    // Should have suggestions
    let suggestions = structured.get("suggestions").and_then(|v| v.as_array());
    assert!(
        suggestions.is_some(),
        "Expected suggestions array in response"
    );
}

/// Tests for suggest_new_skills_tool - respects focus_areas
/// GIVEN a SkillService
/// WHEN suggest_new_skills_tool is called with focus_areas
/// THEN it should include those areas in the analysis
#[test]
fn test_suggest_new_skills_respects_focus_areas() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let project_dir = temp.path().join("project");
    fs::create_dir_all(&project_dir).unwrap();
    fs::write(project_dir.join("main.rs"), "fn main() {}").unwrap();

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "project_dir": project_dir.to_str().unwrap(),
        "focus_areas": ["security", "performance", "observability"]
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.suggest_new_skills_tool(args).unwrap();

    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();

    let gaps = structured.get("gaps").and_then(|v| v.as_array()).unwrap();

    // Focus areas should be included in gaps
    let gap_areas: Vec<&str> = gaps
        .iter()
        .filter_map(|g| g.get("area").and_then(|v| v.as_str()))
        .collect();

    assert!(
        gap_areas
            .iter()
            .any(|a| a.to_lowercase().contains("security")
                || a.to_lowercase().contains("performance")
                || a.to_lowercase().contains("observability")),
        "Expected focus areas in gaps, got: {:?}",
        gap_areas
    );
}

// -------------------------------------------------------------------------
// Error Path Tests
// -------------------------------------------------------------------------

/// Tests that tools handle empty skills gracefully
/// GIVEN a SkillService with no skills
/// WHEN intelligence tools are called
/// THEN they should handle empty state gracefully
#[test]
fn test_intelligence_tools_handle_empty_skills() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // recommend_skills_smart_tool with empty skills
    let args = json!({
        "prompt": "database",
        "include_usage": false,
        "include_context": false
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.recommend_skills_smart_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));

    // search_skills_fuzzy_tool with empty skills
    let args = json!({
        "query": "database",
        "threshold": 0.3
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    assert_eq!(
        structured.get("total_found").and_then(|v| v.as_u64()),
        Some(0)
    );

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }
}

/// Tests that create_skill handles empirical method gracefully
/// GIVEN a SkillService
/// WHEN create_skill_tool is called with empirical method but no session data
/// THEN it should return appropriate error or preview message
#[test]
fn test_create_skill_empirical_without_sessions() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    let original_home = std::env::var("HOME").ok();
    std::env::set_var("HOME", temp.path());

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({
        "name": "test-skill",
        "description": "A test skill",
        "method": "empirical",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.create_skill_tool_sync(args).unwrap();

    // Should succeed but indicate insufficient data
    let structured = result.structured_content.unwrap();
    let errors = structured.get("errors").and_then(|v| v.as_array());
    let preview = structured.get("preview").and_then(|v| v.as_bool());

    // Either errors about missing sessions or preview mode
    assert!(
        (errors.is_some() && !errors.unwrap().is_empty()) || preview == Some(true),
        "Expected errors or preview mode for empirical without sessions"
    );

    match original_home {
        Some(val) => std::env::set_var("HOME", val),
        None => std::env::remove_var("HOME"),
    }
}
