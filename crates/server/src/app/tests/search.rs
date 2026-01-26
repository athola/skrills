//! Search tests - Fuzzy skill search

use super::super::*;
use serde_json::json;
use skrills_discovery::SkillRoot;
use std::time::Duration;
use tempfile::tempdir;

/// Tests for search_skills_fuzzy_tool - basic functionality
/// GIVEN a SkillService with skills
/// WHEN search_skills_fuzzy is called with an exact match query
/// THEN it should return the matching skill with high similarity
#[test]
fn test_search_skills_fuzzy_exact_match() {
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

/// Tests for search_skills_fuzzy_tool - description matching (0.4.8+)
/// GIVEN a SkillService with skills that have descriptions
/// WHEN search_skills_fuzzy is called with a query matching a description
/// THEN it should find the skill via description and indicate matched_field is Description
#[test]
fn test_search_skills_fuzzy_description_match() {
    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a skill with name that doesn't match query, but description does
    let skill_dir = skills_dir.join("helper");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: helper
description: PostgreSQL database management and operations
---
# Helper Skill
A utility skill for database work.
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Search for "database" - name is "helper" but description contains "database"
    let args = json!({
        "query": "database",
        "threshold": 0.3,
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();

    // Should find the skill via description
    assert!(!result.is_error.unwrap_or(true));
    let structured = result.structured_content.unwrap();
    let total = structured
        .get("total_found")
        .and_then(|v| v.as_u64())
        .unwrap();
    assert_eq!(
        total, 1,
        "Expected exactly 1 skill via description match, got {} results",
        total
    );

    let results = structured.get("results").unwrap().as_array().unwrap();
    let first = &results[0];

    // Verify it matched via description, not name
    let matched_field = first.get("matched_field").and_then(|v| v.as_str()).unwrap();
    assert_eq!(
        matched_field, "Description",
        "Expected match via Description, got '{}'",
        matched_field
    );

    // Verify the name is "helper" (not "database")
    let name = first.get("name").and_then(|v| v.as_str()).unwrap();
    assert!(
        name.contains("helper"),
        "Expected name to contain 'helper', got '{}'",
        name
    );
}

/// Tests for search_skills_fuzzy_tool - name priority over description
/// GIVEN a SkillService with skills where name and description both could match
/// WHEN search_skills_fuzzy is called with a query matching the name better
/// THEN it should prefer the name match
#[test]
fn test_search_skills_fuzzy_name_priority_over_description() {
    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a skill where name matches query exactly
    let skill_dir = skills_dir.join("database");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: database
description: Code analysis utilities
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

    // Search for "database" - should match name not description
    let args = json!({
        "query": "database",
        "threshold": 0.3,
        "limit": 10
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();
    let structured = result.structured_content.unwrap();
    let results = structured.get("results").unwrap().as_array().unwrap();
    let first = &results[0];

    // Verify it matched via name (name match should take priority)
    let matched_field = first.get("matched_field").and_then(|v| v.as_str()).unwrap();
    assert_eq!(
        matched_field, "Name",
        "Expected match via Name, got '{}'",
        matched_field
    );
}

/// Tests for search_skills_fuzzy_tool - include_description=false
/// GIVEN a SkillService with skills having descriptions
/// WHEN search_skills_fuzzy is called with include_description=false
/// THEN it should only match on names, not descriptions
#[test]
fn test_search_skills_fuzzy_exclude_description() {
    let temp = tempdir().unwrap();
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();

    // Create a skill where the description contains "kubernetes" but name doesn't
    let skill_dir = skills_dir.join("container-orchestration");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: container-orchestration
description: Kubernetes deployment and management utilities
---
# Container Orchestration Skill
"#,
    )
    .unwrap();

    let roots = vec![SkillRoot {
        root: skills_dir.clone(),
        source: skrills_discovery::SkillSource::Extra(0),
    }];

    let service = SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap();
    service.invalidate_cache().unwrap();

    // Search for "kubernetes" with include_description=false
    // Should NOT find the skill since name doesn't contain "kubernetes"
    let args = json!({
        "query": "kubernetes",
        "threshold": 0.3,
        "limit": 10,
        "include_description": false
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.search_skills_fuzzy_tool(args).unwrap();
    let structured = result.structured_content.unwrap();
    let results = structured.get("results").unwrap().as_array().unwrap();

    // Should be empty since we're not matching descriptions
    assert!(
        results.is_empty(),
        "Expected no results when include_description=false, but got {}",
        results.len()
    );

    // Now search with include_description=true (default)
    // Should find the skill via description
    let args_with_desc = json!({
        "query": "kubernetes",
        "threshold": 0.3,
        "limit": 10,
        "include_description": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let result_with_desc = service.search_skills_fuzzy_tool(args_with_desc).unwrap();
    let structured_with_desc = result_with_desc.structured_content.unwrap();
    let results_with_desc = structured_with_desc
        .get("results")
        .unwrap()
        .as_array()
        .unwrap();

    assert!(
        !results_with_desc.is_empty(),
        "Expected results when include_description=true"
    );
}
