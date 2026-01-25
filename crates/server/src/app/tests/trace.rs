//! Trace tests - Skill loading status and trace instrumentation

use super::super::*;
use serde_json::json;
use std::time::Duration;
use tempfile::tempdir;

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

/// Tests for parse_trace_target case sensitivity
/// GIVEN uppercase and mixed-case target values
/// WHEN parse_trace_target is called
/// THEN it should treat them as invalid and default to Both
/// NOTE: This documents intentional case-sensitive matching behavior
#[test]
fn test_parse_trace_target_case_sensitivity() {
    // Uppercase "CLAUDE" should NOT match "claude"
    let args = json!({"target": "CLAUDE"}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(
        format!("{:?}", target),
        "Both",
        "Uppercase CLAUDE should default to Both (case-sensitive matching)"
    );

    // Mixed case "Claude" should NOT match "claude"
    let args = json!({"target": "Claude"}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(
        format!("{:?}", target),
        "Both",
        "Mixed case Claude should default to Both (case-sensitive matching)"
    );

    // Uppercase "CODEX" should NOT match "codex"
    let args = json!({"target": "CODEX"}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(
        format!("{:?}", target),
        "Both",
        "Uppercase CODEX should default to Both (case-sensitive matching)"
    );

    // Uppercase "BOTH" should NOT match "both" but still default to Both
    let args = json!({"target": "BOTH"}).as_object().cloned().unwrap();
    let target = SkillService::parse_trace_target(&args);
    assert_eq!(
        format!("{:?}", target),
        "Both",
        "Uppercase BOTH should default to Both (wildcard default)"
    );
}

/// Tests for skill_loading_status_tool
/// GIVEN a SkillService
/// WHEN skill_loading_status_tool is called
/// THEN it should return status with structured content
#[test]
fn test_skill_loading_status_tool_returns_status() {
    let _guard = crate::test_support::env_guard();
    let temp = tempdir().unwrap();

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "both"}).as_object().cloned().unwrap();
    let result = service.skill_loading_status_tool(args).unwrap();

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

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

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

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "both", "dry_run": false})
        .as_object()
        .cloned()
        .unwrap();

    let result = service.skill_loading_selftest_tool(args).unwrap();

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

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "claude", "dry_run": true})
        .as_object()
        .cloned()
        .unwrap();

    let result = service.skill_loading_selftest_tool(args).unwrap();

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

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "both", "dry_run": true})
        .as_object()
        .cloned()
        .unwrap();

    let result = service.disable_skill_trace_tool(args).unwrap();

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

    // Use RAII guard for HOME env var - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", Some(temp.path().to_str().unwrap()));

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    for target in ["claude", "codex", "both"] {
        let args = json!({"target": target, "dry_run": true})
            .as_object()
            .cloned()
            .unwrap();

        let result = service.disable_skill_trace_tool(args);

        assert!(
            result.is_ok(),
            "disable_skill_trace_tool should accept target '{}'",
            target
        );
    }
}

/// Tests for enable_skill_trace_tool - integration test for file operations
/// GIVEN a SkillService with a skills directory
/// WHEN enable_skill_trace_tool is called
/// THEN it should create trace skill files and instrument skill files
#[test]
fn test_enable_skill_trace_tool_creates_trace_files() {
    use crate::test_support::TestFixture;

    let _guard = crate::test_support::env_guard();
    let fixture = TestFixture::new().unwrap();

    // Create a sample skill to be instrumented
    fixture
        .create_skill_with_frontmatter(
            "test-skill",
            "A test skill",
            "# Test Skill\nThis is a test skill content.\n",
        )
        .unwrap();

    // Set HOME to fixture's temp directory
    let _home_guard = fixture.home_guard();

    let service =
        SkillService::new_with_ttl(vec![fixture.claude_skills.clone()], Duration::from_secs(1))
            .unwrap();

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
    let trace_skill_path = fixture.claude_skills.join("skrills-skill-trace/SKILL.md");
    assert!(
        trace_skill_path.exists(),
        "Expected trace skill file to be created at {:?}",
        trace_skill_path
    );

    // Verify the probe skill file exists
    // Uses constant from skill_trace.rs: PROBE_SKILL_DIR = "skrills-skill-probe"
    let probe_skill_path = fixture.claude_skills.join("skrills-skill-probe/SKILL.md");
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
    use crate::test_support::TestFixture;

    let _guard = crate::test_support::env_guard();
    let fixture = TestFixture::new().unwrap();

    fixture
        .create_skill("test-skill", "---\nname: test-skill\n---\n# Test\n")
        .unwrap();

    let _home_guard = fixture.home_guard();

    let service =
        SkillService::new_with_ttl(vec![fixture.claude_skills.clone()], Duration::from_secs(1))
            .unwrap();

    let args = json!({
        "target": "claude",
        "dry_run": true
    })
    .as_object()
    .cloned()
    .unwrap();

    let result = service.enable_skill_trace_tool(args).unwrap();

    assert!(!result.is_error.unwrap_or(true));

    // In dry_run mode, trace skill files should NOT be created
    let trace_skill_path = fixture.claude_skills.join("skill-loading-trace/SKILL.md");
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

    // Use RAII guards to unset env vars - automatic cleanup on drop
    let _home_guard = crate::test_support::set_env_var("HOME", None);
    let _user_profile_guard = crate::test_support::set_env_var("USERPROFILE", None);

    let service = SkillService::new_with_ttl(Vec::new(), Duration::from_secs(1)).unwrap();

    let args = json!({"target": "both"}).as_object().cloned().unwrap();
    let result = service.skill_loading_status_tool(args);

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
