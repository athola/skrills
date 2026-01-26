//! Resource tests - Resource reading with dependency resolution

use super::super::*;
use serde_json::json;
use skrills_discovery::SkillRoot;
use std::time::Duration;
use tempfile::tempdir;

#[test]
fn test_read_resource_without_resolve() {
    let temp = tempdir().expect("create temp directory");
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).expect("create skills directory");

    // Create skill A (depends on B)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).expect("create skill-a directory");
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
    .expect("write skill-a");

    // Create skill B (no dependencies)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).expect("create skill-b directory");
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
    .expect("write skill-b");

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60))
        .expect("create skill service");

    // Test reading without resolve param
    let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md";
    let result = service
        .read_resource_sync(skill_a_uri)
        .expect("read resource");

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
        let meta = meta.as_ref().expect("metadata should exist");
        assert_eq!(meta.get("role").and_then(|v| v.as_str()), Some("requested"));
    } else {
        panic!("Expected TextResourceContents");
    }
}

#[test]
fn test_read_resource_with_resolve_true() {
    let temp = tempdir().expect("create temp directory");
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).expect("create skills directory");

    // Create skill A (depends on B and C)
    let skill_a_dir = skills_dir.join("skill-a");
    fs::create_dir_all(&skill_a_dir).expect("create skill-a directory");
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
    .expect("write skill-a");

    // Create skill B (depends on D)
    let skill_b_dir = skills_dir.join("skill-b");
    fs::create_dir_all(&skill_b_dir).expect("create skill-b directory");
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
    .expect("write skill-b");

    // Create skill C (depends on D)
    let skill_c_dir = skills_dir.join("skill-c");
    fs::create_dir_all(&skill_c_dir).expect("create skill-c directory");
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
    .expect("write skill-c");

    // Create skill D (no dependencies)
    let skill_d_dir = skills_dir.join("skill-d");
    fs::create_dir_all(&skill_d_dir).expect("create skill-d directory");
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
    .expect("write skill-d");

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60))
        .expect("create skill service");

    // Test reading with resolve=true
    let skill_a_uri = "skill://skrills/extra0/skill-a/SKILL.md?resolve=true";
    let result = service
        .read_resource_sync(skill_a_uri)
        .expect("read resource with resolve");

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
        let meta = meta.as_ref().expect("metadata should exist");
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
            let meta = meta.as_ref().expect("dependency metadata should exist");
            assert_eq!(
                meta.get("role").and_then(|v| v.as_str()),
                Some("dependency")
            );
        }
    }
}

#[test]
fn test_read_resource_with_resolve_false() {
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

/// Tests for recommend_skills method - URI validation
/// GIVEN a SkillService with valid skills
/// WHEN recommend_skills is called with a non-existent URI
/// THEN it should return an error indicating the skill was not found
#[test]
fn test_recommend_skills_uri_not_found_returns_error() {
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

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

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
