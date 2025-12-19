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
