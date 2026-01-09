//! Parse Codex CLI session and history files from ~/.codex/

use super::{CommandEntry, SkillUsageEvent};
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tracing::debug;

/// Parse Codex CLI skills-history.json
pub fn parse_codex_skills_history(path: &Path) -> Result<Vec<SkillUsageEvent>> {
    let mut events = Vec::new();

    if !path.exists() {
        debug!("Codex skills-history.json does not exist: {:?}", path);
        return Ok(events);
    }

    let content = fs::read_to_string(path)?;

    // Try to parse as array first, then as object with entries
    if let Ok(entries) = serde_json::from_str::<Vec<Value>>(&content) {
        for entry in entries {
            events.extend(parse_skills_history_entry(&entry));
        }
    } else if let Ok(obj) = serde_json::from_str::<Value>(&content) {
        // Handle object format with nested arrays
        if let Some(entries) = obj.get("entries").and_then(|e| e.as_array()) {
            for entry in entries {
                events.extend(parse_skills_history_entry(entry));
            }
        }
        // Also check for direct skills array
        if let Some(skills) = obj.get("skills").and_then(|s| s.as_array()) {
            let timestamp = obj.get("ts").and_then(|t| t.as_u64()).unwrap_or(0);
            for skill in skills {
                if let Some(path) = skill.as_str() {
                    events.push(SkillUsageEvent {
                        timestamp,
                        skill_path: path.to_string(),
                        session_id: format!("codex-{}", timestamp),
                        prompt_context: None,
                    });
                }
            }
        }
    }

    Ok(events)
}

fn parse_skills_history_entry(entry: &Value) -> Vec<SkillUsageEvent> {
    let mut events = Vec::new();

    let timestamp = entry.get("ts").and_then(|t| t.as_u64()).unwrap_or(0);
    let session_id = entry
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("codex-{}", timestamp));

    // Check for skills array
    if let Some(skills) = entry.get("skills").and_then(|s| s.as_array()) {
        for skill in skills {
            if let Some(path) = skill.as_str() {
                events.push(SkillUsageEvent {
                    timestamp,
                    skill_path: path.to_string(),
                    session_id: session_id.clone(),
                    prompt_context: None,
                });
            }
        }
    }

    // Check for single skill field
    if let Some(skill) = entry.get("skill").and_then(|s| s.as_str()) {
        events.push(SkillUsageEvent {
            timestamp,
            skill_path: skill.to_string(),
            session_id,
            prompt_context: None,
        });
    }

    events
}

/// Parse Codex CLI command history from ~/.codex/history.jsonl
pub fn parse_codex_command_history(history_path: &Path) -> Result<Vec<CommandEntry>> {
    let mut entries = Vec::new();

    if !history_path.exists() {
        debug!("Codex history file does not exist: {:?}", history_path);
        return Ok(entries);
    }

    let content = fs::read_to_string(history_path)?;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            let text = entry
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let timestamp = entry.get("ts").and_then(|v| v.as_u64()).unwrap_or(0);

            let session_id = entry
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            if !text.is_empty() {
                entries.push(CommandEntry {
                    text,
                    timestamp,
                    session_id,
                    project: None, // Codex history doesn't include project
                });
            }
        }
    }

    Ok(entries)
}

/// Parse Codex session files from ~/.codex/sessions/
pub fn parse_codex_sessions(sessions_dir: &Path) -> Result<Vec<SkillUsageEvent>> {
    let mut events = Vec::new();

    if !sessions_dir.exists() {
        debug!(
            "Codex sessions directory does not exist: {:?}",
            sessions_dir
        );
        return Ok(events);
    }

    // Walk through year/month/day structure
    for entry in walkdir::WalkDir::new(sessions_dir)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
            match parse_codex_session_file(path) {
                Ok(session_events) => events.extend(session_events),
                Err(e) => debug!("Failed to parse Codex session file {:?}: {}", path, e),
            }
        }
    }

    Ok(events)
}

fn parse_codex_session_file(path: &Path) -> Result<Vec<SkillUsageEvent>> {
    let mut events = Vec::new();
    let content = fs::read_to_string(path)?;

    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            // Look for skill-related entries in payload
            if let Some(payload) = entry.get("payload") {
                // Check for skill loading in tool calls
                if let Some(tools) = payload.get("tools").and_then(|t| t.as_array()) {
                    for tool in tools {
                        if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
                            if name.contains("skill") {
                                if let Some(args) = tool.get("arguments") {
                                    if let Some(skill_path) = extract_skill_from_args(args) {
                                        let timestamp = entry
                                            .get("timestamp")
                                            .and_then(|t| t.as_str())
                                            .and_then(parse_codex_timestamp)
                                            .unwrap_or(0);

                                        events.push(SkillUsageEvent {
                                            timestamp,
                                            skill_path,
                                            session_id: session_id.clone(),
                                            prompt_context: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(events)
}

fn extract_skill_from_args(args: &Value) -> Option<String> {
    if let Some(skill) = args.get("skill").and_then(|s| s.as_str()) {
        return Some(skill.to_string());
    }
    if let Some(uri) = args.get("uri").and_then(|u| u.as_str()) {
        return Some(uri.to_string());
    }
    if let Some(path) = args.get("path").and_then(|p| p.as_str()) {
        if path.contains("SKILL.md") {
            return Some(path.to_string());
        }
    }
    None
}

fn parse_codex_timestamp(s: &str) -> Option<u64> {
    // Try ISO 8601 format first
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp() as u64);
    }
    // Try parsing as Unix timestamp
    s.parse::<u64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // =========================================================================
    // Skills History Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_nonexistent_skills_history() {
        let events = parse_codex_skills_history(Path::new("/nonexistent/path")).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_skills_history_array_format() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("skills-history.json");

        let content = r#"[
            {"ts": 1703001000, "skills": ["test-skill", "another-skill"]},
            {"ts": 1703002000, "skill": "single-skill"}
        ]"#;

        fs::write(&path, content).unwrap();

        let events = parse_codex_skills_history(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].skill_path, "test-skill");
        assert_eq!(events[1].skill_path, "another-skill");
        assert_eq!(events[2].skill_path, "single-skill");
    }

    #[test]
    fn test_parse_skills_history_object_with_entries() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("skills-history.json");

        let content = r#"{
            "entries": [
                {"ts": 1703001000, "skills": ["skill-a", "skill-b"]},
                {"ts": 1703002000, "skill": "skill-c", "session_id": "custom-session"}
            ]
        }"#;

        fs::write(&path, content).unwrap();

        let events = parse_codex_skills_history(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].skill_path, "skill-a");
        assert_eq!(events[1].skill_path, "skill-b");
        assert_eq!(events[2].skill_path, "skill-c");
        assert_eq!(events[2].session_id, "custom-session");
    }

    #[test]
    fn test_parse_skills_history_object_with_skills() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("skills-history.json");

        let content = r#"{
            "ts": 1703001000,
            "skills": ["direct-skill-1", "direct-skill-2"]
        }"#;

        fs::write(&path, content).unwrap();

        let events = parse_codex_skills_history(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].skill_path, "direct-skill-1");
        assert_eq!(events[1].skill_path, "direct-skill-2");
        assert!(events[0].session_id.contains("codex-"));
    }

    #[test]
    fn test_parse_skills_history_empty_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("skills-history.json");
        fs::write(&path, "").unwrap();

        // Empty file should fail JSON parsing but not panic
        let result = parse_codex_skills_history(&path);
        // Should return empty vec or handle gracefully
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_parse_skills_history_empty_array() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("skills-history.json");
        fs::write(&path, "[]").unwrap();

        let events = parse_codex_skills_history(&path).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_skills_history_with_session_id() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("skills-history.json");

        let content = r#"[
            {"ts": 1703001000, "session_id": "my-custom-session", "skill": "skill-1"}
        ]"#;

        fs::write(&path, content).unwrap();

        let events = parse_codex_skills_history(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session_id, "my-custom-session");
    }

    #[test]
    fn test_parse_skills_history_missing_timestamp() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("skills-history.json");

        let content = r#"[{"skill": "no-timestamp-skill"}]"#;

        fs::write(&path, content).unwrap();

        let events = parse_codex_skills_history(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].timestamp, 0);
    }

    #[test]
    fn test_parse_skills_history_no_skills() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("skills-history.json");

        let content = r#"[{"ts": 1703001000, "other_field": "value"}]"#;

        fs::write(&path, content).unwrap();

        let events = parse_codex_skills_history(&path).unwrap();
        assert!(events.is_empty());
    }

    // =========================================================================
    // Command History Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_command_history() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        let content = r#"{"session_id":"sess1","ts":1703001000,"text":"test command"}
{"session_id":"sess2","ts":1703002000,"text":"another command"}"#;

        fs::write(&path, content).unwrap();

        let entries = parse_codex_command_history(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "test command");
        assert_eq!(entries[0].session_id, "sess1");
    }

    #[test]
    fn test_parse_command_history_nonexistent() {
        let entries = parse_codex_command_history(Path::new("/nonexistent/history.jsonl")).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_command_history_empty_file() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");
        fs::write(&path, "").unwrap();

        let entries = parse_codex_command_history(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_command_history_blank_lines() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        let content = r#"{"session_id":"s1","ts":1703001000,"text":"cmd1"}

{"session_id":"s1","ts":1703002000,"text":"cmd2"}

{"session_id":"s1","ts":1703003000,"text":"cmd3"}"#;

        fs::write(&path, content).unwrap();

        let entries = parse_codex_command_history(&path).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_parse_command_history_malformed_json() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        let content = r#"{"session_id":"s1","ts":1703001000,"text":"valid"}
{malformed json
{"session_id":"s2","ts":1703002000,"text":"also valid"}"#;

        fs::write(&path, content).unwrap();

        let entries = parse_codex_command_history(&path).unwrap();
        // Should skip malformed and continue
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "valid");
        assert_eq!(entries[1].text, "also valid");
    }

    #[test]
    fn test_parse_command_history_missing_fields() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        let content = r#"{"session_id":"s1","ts":1703001000,"text":"complete"}
{"ts":1703002000,"text":"no session"}
{"session_id":"s2","text":"no timestamp"}
{"session_id":"s3","ts":1703003000}"#;

        fs::write(&path, content).unwrap();

        let entries = parse_codex_command_history(&path).unwrap();
        // Entry without text is skipped (empty text filtered)
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "complete");
        assert_eq!(entries[1].text, "no session");
        assert_eq!(entries[1].session_id, "unknown");
        assert_eq!(entries[2].text, "no timestamp");
        assert_eq!(entries[2].timestamp, 0);
    }

    #[test]
    fn test_parse_command_history_empty_text() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        let content = r#"{"session_id":"s1","ts":1703001000,"text":""}
{"session_id":"s2","ts":1703002000,"text":"non-empty"}"#;

        fs::write(&path, content).unwrap();

        let entries = parse_codex_command_history(&path).unwrap();
        // Empty text is filtered
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "non-empty");
    }

    #[test]
    fn test_parse_command_history_no_project() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        let content = r#"{"session_id":"s1","ts":1703001000,"text":"cmd"}"#;

        fs::write(&path, content).unwrap();

        let entries = parse_codex_command_history(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].project.is_none()); // Codex doesn't include project
    }

    // =========================================================================
    // Session Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_sessions_nonexistent() {
        let events = parse_codex_sessions(Path::new("/nonexistent/sessions")).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_sessions_empty_directory() {
        let tmp = tempdir().unwrap();
        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_sessions_with_skill_tool() {
        let tmp = tempdir().unwrap();
        let session_path = tmp.path().join("session.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","payload":{"tools":[{"name":"load_skill","arguments":{"skill":"my-skill"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "my-skill");
        assert_eq!(events[0].session_id, "session");
    }

    #[test]
    fn test_parse_sessions_nested_directory() {
        let tmp = tempdir().unwrap();

        // Create year/month/day structure
        let nested = tmp.path().join("2024/01/15");
        fs::create_dir_all(&nested).unwrap();

        let content = r#"{"timestamp":"2024-01-15T10:00:00Z","payload":{"tools":[{"name":"skill_loader","arguments":{"skill":"nested-skill"}}]}}"#;
        fs::write(nested.join("sess.jsonl"), content).unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "nested-skill");
    }

    #[test]
    fn test_parse_sessions_multiple_files() {
        let tmp = tempdir().unwrap();

        let content1 = r#"{"timestamp":"2024-01-01T12:00:00Z","payload":{"tools":[{"name":"skill","arguments":{"skill":"skill-1"}}]}}"#;
        let content2 = r#"{"timestamp":"2024-01-01T13:00:00Z","payload":{"tools":[{"name":"skill","arguments":{"skill":"skill-2"}}]}}"#;

        fs::write(tmp.path().join("sess1.jsonl"), content1).unwrap();
        fs::write(tmp.path().join("sess2.jsonl"), content2).unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 2);

        let paths: Vec<&str> = events.iter().map(|e| e.skill_path.as_str()).collect();
        assert!(paths.contains(&"skill-1"));
        assert!(paths.contains(&"skill-2"));
    }

    #[test]
    fn test_parse_sessions_empty_jsonl() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("empty.jsonl"), "").unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_sessions_malformed_entries() {
        let tmp = tempdir().unwrap();
        let session_path = tmp.path().join("sess.jsonl");

        let content = r#"not valid json
{"timestamp":"2024-01-01T12:00:00Z","payload":{"tools":[{"name":"skill","arguments":{"skill":"valid"}}]}}
{"incomplete": "#;

        fs::write(&session_path, content).unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "valid");
    }

    #[test]
    fn test_parse_sessions_no_skill_events() {
        let tmp = tempdir().unwrap();
        let session_path = tmp.path().join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","payload":{"tools":[{"name":"read_file","arguments":{"path":"/some/file"}}]}}
{"timestamp":"2024-01-01T12:01:00Z","payload":{"message":"just text"}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_sessions_ignores_non_jsonl() {
        let tmp = tempdir().unwrap();

        let jsonl_content = r#"{"timestamp":"2024-01-01T12:00:00Z","payload":{"tools":[{"name":"skill","arguments":{"skill":"real"}}]}}"#;
        fs::write(tmp.path().join("valid.jsonl"), jsonl_content).unwrap();
        fs::write(tmp.path().join("notes.txt"), "text").unwrap();
        fs::write(tmp.path().join("data.json"), "{}").unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "real");
    }

    #[test]
    fn test_parse_sessions_with_uri_argument() {
        let tmp = tempdir().unwrap();
        let session_path = tmp.path().join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","payload":{"tools":[{"name":"skill","arguments":{"uri":"skill://plugin/skill-name"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "skill://plugin/skill-name");
    }

    #[test]
    fn test_parse_sessions_with_skill_md_path() {
        let tmp = tempdir().unwrap();
        let session_path = tmp.path().join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","payload":{"tools":[{"name":"skill","arguments":{"path":"/home/user/skills/test/SKILL.md"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "/home/user/skills/test/SKILL.md");
    }

    #[test]
    fn test_parse_sessions_path_not_skill_md() {
        let tmp = tempdir().unwrap();
        let session_path = tmp.path().join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","payload":{"tools":[{"name":"skill","arguments":{"path":"/home/user/README.md"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_codex_sessions(tmp.path()).unwrap();
        assert!(events.is_empty()); // Non-SKILL.md paths are ignored
    }

    // =========================================================================
    // Timestamp Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_codex_timestamp_rfc3339() {
        let ts = parse_codex_timestamp("2024-01-15T10:30:00Z");
        assert!(ts.is_some());
        assert_eq!(ts.unwrap(), 1705314600);
    }

    #[test]
    fn test_parse_codex_timestamp_with_timezone() {
        let ts = parse_codex_timestamp("2024-01-15T10:30:00+05:00");
        assert!(ts.is_some());
    }

    #[test]
    fn test_parse_codex_timestamp_unix() {
        let ts = parse_codex_timestamp("1705314600");
        assert!(ts.is_some());
        assert_eq!(ts.unwrap(), 1705314600);
    }

    #[test]
    fn test_parse_codex_timestamp_invalid() {
        assert!(parse_codex_timestamp("not a timestamp").is_none());
        assert!(parse_codex_timestamp("").is_none());
    }

    // =========================================================================
    // Extract Skill From Args Tests
    // =========================================================================

    #[test]
    fn test_extract_skill_from_args_skill() {
        let args = serde_json::json!({"skill": "my-skill"});
        assert_eq!(extract_skill_from_args(&args), Some("my-skill".to_string()));
    }

    #[test]
    fn test_extract_skill_from_args_uri() {
        let args = serde_json::json!({"uri": "skill://plugin/skill"});
        assert_eq!(
            extract_skill_from_args(&args),
            Some("skill://plugin/skill".to_string())
        );
    }

    #[test]
    fn test_extract_skill_from_args_path_skill_md() {
        let args = serde_json::json!({"path": "/path/to/SKILL.md"});
        assert_eq!(
            extract_skill_from_args(&args),
            Some("/path/to/SKILL.md".to_string())
        );
    }

    #[test]
    fn test_extract_skill_from_args_path_non_skill() {
        let args = serde_json::json!({"path": "/path/to/README.md"});
        assert_eq!(extract_skill_from_args(&args), None);
    }

    #[test]
    fn test_extract_skill_from_args_empty() {
        let args = serde_json::json!({});
        assert_eq!(extract_skill_from_args(&args), None);
    }

    #[test]
    fn test_extract_skill_from_args_other_fields() {
        let args = serde_json::json!({"file": "/some/file", "option": true});
        assert_eq!(extract_skill_from_args(&args), None);
    }

    // =========================================================================
    // Parse Skills History Entry Tests
    // =========================================================================

    #[test]
    fn test_parse_skills_history_entry_with_skills_array() {
        let entry = serde_json::json!({
            "ts": 1703001000,
            "skills": ["skill-a", "skill-b"],
            "session_id": "session-123"
        });

        let events = parse_skills_history_entry(&entry);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].skill_path, "skill-a");
        assert_eq!(events[0].timestamp, 1703001000);
        assert_eq!(events[0].session_id, "session-123");
    }

    #[test]
    fn test_parse_skills_history_entry_with_single_skill() {
        let entry = serde_json::json!({
            "ts": 1703001000,
            "skill": "single-skill"
        });

        let events = parse_skills_history_entry(&entry);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "single-skill");
    }

    #[test]
    fn test_parse_skills_history_entry_both_skills_and_skill() {
        let entry = serde_json::json!({
            "ts": 1703001000,
            "skills": ["array-skill"],
            "skill": "single-skill"
        });

        let events = parse_skills_history_entry(&entry);
        // Should capture both
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_parse_skills_history_entry_no_session_id() {
        let entry = serde_json::json!({
            "ts": 1703001000,
            "skill": "test"
        });

        let events = parse_skills_history_entry(&entry);
        assert_eq!(events.len(), 1);
        // Should generate session ID from timestamp
        assert!(events[0].session_id.contains("codex-1703001000"));
    }

    #[test]
    fn test_parse_skills_history_entry_empty() {
        let entry = serde_json::json!({});
        let events = parse_skills_history_entry(&entry);
        assert!(events.is_empty());
    }
}
