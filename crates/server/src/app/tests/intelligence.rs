//! Intelligence tests - Smart recommendations, skill creation, GitHub search

use super::super::*;
use serde_json::json;
use skrills_discovery::SkillRoot;
use std::time::Duration;
use tempfile::tempdir;

// -------------------------------------------------------------------------
// recommend_skills_smart_tool Tests
// -------------------------------------------------------------------------

/// Tests for recommend_skills_smart_tool - basic functionality
/// GIVEN a SkillService with skills
/// WHEN recommend_skills_smart_tool is called
/// THEN it should return recommendations with structured content
#[test]
fn test_recommend_skills_smart_tool_basic() {
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

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    // Empty args should work with defaults
    let args = json!({}).as_object().cloned().unwrap();

    let result = service.recommend_skills_smart_tool(args);

    // Should not error even with no skills
    result.expect("recommend_skills_smart_tool with defaults should succeed");
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

/// GIVEN a home with no Claude session history
/// WHEN create_skill_tool is called with the empirical method
/// THEN the tool result is flagged as a tool-level error
#[test]
fn create_skill_reports_a_tool_error_when_empirical_generation_finds_no_sessions() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();
    let args = json!({
        "name": "no-sessions",
        "description": "a skill built from behaviour that was never recorded",
        "method": "empirical",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service
        .create_skill_tool_sync(args)
        .expect("empirical generation reports failure in the result, not as an Err");

    assert_eq!(
        result.is_error,
        Some(true),
        "the failed-creation branch must set is_error"
    );
    let structured = result
        .structured_content
        .expect("structured content should be present");
    assert_eq!(structured["success"], json!(false));
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
        // Rate limit errors are expected network failures, not validation errors
        let is_rate_limit = err_msg.contains("rate limit");
        let is_validation_error =
            err_msg.to_lowercase().contains("missing") || err_msg.contains("required");
        assert!(
            is_rate_limit || !is_validation_error,
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
    result.expect("analyze_project_context_tool should respect options");
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

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

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
}

/// Tests that create_skill handles empirical method gracefully
/// GIVEN a SkillService
/// WHEN create_skill_tool is called with empirical method but no session data
/// THEN it should return appropriate error or preview message
#[test]
fn test_create_skill_empirical_without_sessions() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

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
}

// -------------------------------------------------------------------------
// Gap detection and limits (SA-38, SA-39)
// -------------------------------------------------------------------------

/// SA-38: short language names match whole tokens only.
#[test]
fn name_covers_matches_whole_tokens() {
    use super::super::intelligence::name_covers;
    assert!(name_covers("go-testing/SKILL.md", "go"));
    assert!(!name_covers("django-patterns/SKILL.md", "go"));
    assert!(!name_covers("rust-best-practices/SKILL.md", "r"));
    assert!(!name_covers("commit-helper/SKILL.md", "c"));
    assert!(name_covers("c-best-practices/SKILL.md", "c++"));
    assert!(name_covers("next-js-patterns/SKILL.md", "next.js"));
    assert!(!name_covers("anything", ""));
}

/// SA-39: caller-supplied limits are bounded.
#[test]
fn clamp_limit_bounds_requests() {
    use super::super::intelligence::clamp_limit;
    assert_eq!(clamp_limit(None, 10), 10);
    assert_eq!(clamp_limit(Some(0), 10), 1);
    assert_eq!(clamp_limit(Some(5), 10), 5);
    assert_eq!(clamp_limit(Some(u64::MAX), 10), 100);
}

/// SA-38: a project whose primary language is Go reports a gap even when
/// an unrelated skill name contains the letters "go".
#[test]
fn suggest_new_skills_does_not_count_substring_matches() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _cache = crate::test_support::set_env_var(
        "SKRILLS_CACHE_PATH",
        Some(temp.path().join("cache.json").to_str().unwrap()),
    );
    let project = temp.path().join("project");
    fs::create_dir_all(&project).unwrap();
    for i in 0..5 {
        fs::write(project.join(format!("m{i}.go")), "package main\n").unwrap();
    }
    let skills_dir = temp.path().join("skills");
    let skill = skills_dir.join("django-patterns");
    fs::create_dir_all(&skill).unwrap();
    fs::write(
        skill.join("SKILL.md"),
        "---\nname: django-patterns\ndescription: d\n---\n",
    )
    .unwrap();
    let service = service_with_claude_root(&skills_dir);

    let result = service
        .suggest_new_skills_tool(
            json!({"project_dir": project.display().to_string()})
                .as_object()
                .cloned()
                .unwrap(),
        )
        .unwrap();
    let structured = result.structured_content.unwrap();
    let suggestions = structured["suggestions"].as_array().unwrap();
    assert!(
        suggestions.iter().any(|s| s == "go-best-practices"),
        "{structured}"
    );
}

// -------------------------------------------------------------------------
// create_skill target_dir containment (SA-1)
// -------------------------------------------------------------------------

fn run_mcp_create_skill(
    service: &SkillService,
    args: serde_json::Map<String, serde_json::Value>,
) -> Result<rmcp::model::CallToolResult> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(service.create_skill_tool(args))
}

fn service_with_claude_root(skills_dir: &std::path::Path) -> SkillService {
    let roots = vec![SkillRoot {
        root: skills_dir.to_path_buf(),
        source: skrills_discovery::SkillSource::Claude,
    }];
    SkillService::new_with_roots_for_test(roots, Duration::from_secs(60)).unwrap()
}

/// GIVEN an MCP caller
/// WHEN create-skill names a target_dir outside every writable skill root
/// THEN the request is refused before any generation runs
#[test]
fn mcp_create_skill_rejects_target_dir_outside_skill_roots() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _cache = crate::test_support::set_env_var(
        "SKRILLS_CACHE_PATH",
        Some(temp.path().join("cache.json").to_str().unwrap()),
    );
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let service = service_with_claude_root(&skills_dir);

    let outside = temp.path().join("autostart");
    for target in [
        outside.display().to_string(),
        skills_dir.join("../autostart").display().to_string(),
    ] {
        let args = json!({
            "name": "evil",
            "description": "x",
            "method": "github",
            "target_dir": target,
        })
        .as_object()
        .cloned()
        .unwrap();
        let err = run_mcp_create_skill(&service, args)
            .expect_err("target_dir outside the skill roots must be refused");
        assert!(
            err.to_string().contains("target_dir"),
            "unexpected error for {target}: {err}"
        );
    }
    assert!(
        !outside.exists(),
        "nothing may be created outside the roots"
    );
}

/// GIVEN an MCP caller
/// WHEN target_dir sits inside a writable skill root
/// THEN containment does not refuse it
#[test]
fn mcp_create_skill_accepts_target_dir_inside_skill_root() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));
    let _cache = crate::test_support::set_env_var(
        "SKRILLS_CACHE_PATH",
        Some(temp.path().join("cache.json").to_str().unwrap()),
    );
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let service = service_with_claude_root(&skills_dir);

    let args = json!({
        "name": "ok-skill",
        "description": "x",
        "method": "empirical",
        "dry_run": true,
        "target_dir": skills_dir.join("nested").display().to_string(),
    })
    .as_object()
    .cloned()
    .unwrap();
    run_mcp_create_skill(&service, args).expect("target_dir inside a root is allowed");
}

/// GIVEN the CLI wrapper
/// WHEN the user passes an explicit --target-dir anywhere
/// THEN containment does not apply (the user chose the path)
#[test]
fn cli_create_skill_keeps_arbitrary_target_dir() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();
    let _home = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));
    let _cache = crate::test_support::set_env_var(
        "SKRILLS_CACHE_PATH",
        Some(temp.path().join("cache.json").to_str().unwrap()),
    );
    let skills_dir = temp.path().join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    let service = service_with_claude_root(&skills_dir);

    let args = json!({
        "name": "cli-skill",
        "description": "x",
        "method": "empirical",
        "dry_run": true,
        "target_dir": temp.path().join("elsewhere").display().to_string(),
    })
    .as_object()
    .cloned()
    .unwrap();
    service
        .create_skill_tool_sync(args)
        .expect("CLI target_dir is trusted");
}

/// GIVEN an existing SKILL.md
/// WHEN a generated skill is written to the same place
/// THEN the write is refused and the file is untouched
#[test]
fn write_generated_skill_refuses_to_overwrite() {
    let temp = tempdir().unwrap();
    let skill_dir = temp.path().join("demo");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), "mine").unwrap();

    let err = super::super::intelligence::write_generated_skill(temp.path(), "demo", "theirs")
        .expect_err("existing SKILL.md must not be overwritten");
    assert!(err.to_string().contains("already exists"), "{err}");
    assert_eq!(
        fs::read_to_string(skill_dir.join("SKILL.md")).unwrap(),
        "mine"
    );

    let written =
        super::super::intelligence::write_generated_skill(temp.path(), "fresh", "body").unwrap();
    assert_eq!(fs::read_to_string(written).unwrap(), "body");
}
