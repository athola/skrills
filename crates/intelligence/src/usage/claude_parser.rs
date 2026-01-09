//! Parse Claude Code session files from ~/.claude/

use super::{CommandEntry, SkillUsageEvent};
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::Path;
use tracing::debug;

/// Parse Claude Code session files from ~/.claude/projects/
pub fn parse_claude_sessions(projects_dir: &Path) -> Result<Vec<SkillUsageEvent>> {
    let mut events = Vec::new();

    if !projects_dir.exists() {
        debug!(
            "Claude projects directory does not exist: {:?}",
            projects_dir
        );
        return Ok(events);
    }

    for project_entry in fs::read_dir(projects_dir)? {
        let project = project_entry?;
        if !project.file_type()?.is_dir() {
            continue;
        }

        for session_entry in fs::read_dir(project.path())? {
            let session = session_entry?;
            let path = session.path();
            if path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                match parse_claude_session_file(&path) {
                    Ok(session_events) => events.extend(session_events),
                    Err(e) => debug!("Failed to parse session file {:?}: {}", path, e),
                }
            }
        }
    }

    Ok(events)
}

/// Parse Claude Code command history from ~/.claude/history.jsonl
pub fn parse_claude_command_history(history_path: &Path) -> Result<Vec<CommandEntry>> {
    let mut entries = Vec::new();

    if !history_path.exists() {
        debug!("Claude history file does not exist: {:?}", history_path);
        return Ok(entries);
    }

    let content = fs::read_to_string(history_path)?;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            let text = entry
                .get("display")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            let timestamp = entry.get("timestamp").and_then(|v| v.as_u64()).unwrap_or(0) / 1000; // Convert from ms to seconds

            let session_id = entry
                .get("sessionId")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            let project = entry
                .get("project")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            if !text.is_empty() {
                entries.push(CommandEntry {
                    text,
                    timestamp,
                    session_id,
                    project,
                });
            }
        }
    }

    Ok(entries)
}

fn parse_claude_session_file(path: &Path) -> Result<Vec<SkillUsageEvent>> {
    let mut events = Vec::new();
    let content = fs::read_to_string(path)?;
    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut last_user_prompt: Option<String> = None;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<Value>(line) {
            // Track user prompts for context
            if let Some(message) = entry.get("message") {
                if message.get("role").and_then(|r| r.as_str()) == Some("user") {
                    if let Some(contents) = message.get("content").and_then(|c| c.as_array()) {
                        for content_block in contents {
                            if content_block.get("type").and_then(|t| t.as_str()) == Some("text") {
                                if let Some(text) =
                                    content_block.get("text").and_then(|t| t.as_str())
                                {
                                    last_user_prompt =
                                        Some(text.chars().take(200).collect::<String>());
                                }
                            }
                        }
                    }
                }

                // Look for tool_use content blocks
                if let Some(contents) = message.get("content").and_then(|c| c.as_array()) {
                    for content_block in contents {
                        if content_block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            if let Some(name) = content_block.get("name").and_then(|n| n.as_str()) {
                                // Track skill-loading related tools
                                if name.contains("skill") || name == "Skill" {
                                    if let Some(input) = content_block.get("input") {
                                        if let Some(skill_path) = extract_skill_path(input) {
                                            let timestamp = parse_timestamp(
                                                entry.get("timestamp").and_then(|t| t.as_str()),
                                            );

                                            events.push(SkillUsageEvent {
                                                timestamp,
                                                skill_path,
                                                session_id: session_id.clone(),
                                                prompt_context: last_user_prompt.clone(),
                                            });
                                        }
                                    }
                                }
                                // Also track Read tool for SKILL.md files
                                if name == "Read" {
                                    if let Some(input) = content_block.get("input") {
                                        if let Some(file_path) =
                                            input.get("file_path").and_then(|p| p.as_str())
                                        {
                                            if file_path.contains("SKILL.md")
                                                || file_path.contains("/skills/")
                                            {
                                                let timestamp = parse_timestamp(
                                                    entry.get("timestamp").and_then(|t| t.as_str()),
                                                );

                                                events.push(SkillUsageEvent {
                                                    timestamp,
                                                    skill_path: file_path.to_string(),
                                                    session_id: session_id.clone(),
                                                    prompt_context: last_user_prompt.clone(),
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
        }
    }

    Ok(events)
}

fn extract_skill_path(input: &Value) -> Option<String> {
    // Check for skill parameter in Skill tool
    if let Some(skill) = input.get("skill").and_then(|s| s.as_str()) {
        return Some(skill.to_string());
    }
    // Check for uri in skill tools
    if let Some(uri) = input.get("uri").and_then(|u| u.as_str()) {
        if uri.starts_with("skill://") {
            return Some(uri.to_string());
        }
    }
    // Check for file_path
    if let Some(path) = input.get("file_path").and_then(|p| p.as_str()) {
        if path.contains("SKILL.md") {
            return Some(path.to_string());
        }
    }
    None
}

fn parse_timestamp(s: Option<&str>) -> u64 {
    s.and_then(|ts| {
        chrono::DateTime::parse_from_rfc3339(ts)
            .ok()
            .map(|dt| dt.timestamp() as u64)
    })
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    // =========================================================================
    // Directory and Path Tests
    // =========================================================================

    #[test]
    fn test_parse_empty_directory() {
        let tmp = tempdir().unwrap();
        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_nonexistent_directory() {
        let events = parse_claude_sessions(Path::new("/nonexistent/path")).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_nonexistent_history_file() {
        let entries =
            parse_claude_command_history(Path::new("/nonexistent/history.jsonl")).unwrap();
        assert!(entries.is_empty());
    }

    // =========================================================================
    // Command History Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_command_history() {
        let tmp = tempdir().unwrap();
        let history_path = tmp.path().join("history.jsonl");

        let content = r#"{"display":"test command","timestamp":1703001000000,"sessionId":"abc123","project":"/home/user/project"}
{"display":"another command","timestamp":1703002000000,"sessionId":"abc123"}"#;

        fs::write(&history_path, content).unwrap();

        let entries = parse_claude_command_history(&history_path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "test command");
        assert_eq!(entries[0].timestamp, 1703001000);
        assert_eq!(entries[0].project, Some("/home/user/project".to_string()));
        assert_eq!(entries[1].text, "another command");
        assert!(entries[1].project.is_none());
    }

    #[test]
    fn test_parse_command_history_empty_file() {
        let tmp = tempdir().unwrap();
        let history_path = tmp.path().join("history.jsonl");
        fs::write(&history_path, "").unwrap();

        let entries = parse_claude_command_history(&history_path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_command_history_blank_lines() {
        let tmp = tempdir().unwrap();
        let history_path = tmp.path().join("history.jsonl");

        let content = r#"{"display":"command1","timestamp":1703001000000,"sessionId":"s1"}

{"display":"command2","timestamp":1703002000000,"sessionId":"s1"}

{"display":"command3","timestamp":1703003000000,"sessionId":"s1"}"#;

        fs::write(&history_path, content).unwrap();

        let entries = parse_claude_command_history(&history_path).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_parse_command_history_malformed_json() {
        let tmp = tempdir().unwrap();
        let history_path = tmp.path().join("history.jsonl");

        let content = r#"{"display":"valid","timestamp":1703001000000,"sessionId":"s1"}
{invalid json here}
{"display":"also valid","timestamp":1703002000000,"sessionId":"s2"}"#;

        fs::write(&history_path, content).unwrap();

        let entries = parse_claude_command_history(&history_path).unwrap();
        // Should skip malformed line and continue
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].text, "valid");
        assert_eq!(entries[1].text, "also valid");
    }

    #[test]
    fn test_parse_command_history_missing_fields() {
        let tmp = tempdir().unwrap();
        let history_path = tmp.path().join("history.jsonl");

        let content = r#"{"display":"has all","timestamp":1703001000000,"sessionId":"s1","project":"/proj"}
{"timestamp":1703002000000,"sessionId":"s2"}
{"display":"no timestamp","sessionId":"s3"}
{"display":"no session","timestamp":1703003000000}"#;

        fs::write(&history_path, content).unwrap();

        let entries = parse_claude_command_history(&history_path).unwrap();
        // Entry without display text is skipped (empty text filtered)
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].text, "has all");
        assert_eq!(entries[0].session_id, "s1");
        assert_eq!(entries[1].text, "no timestamp");
        assert_eq!(entries[1].timestamp, 0);
        assert_eq!(entries[2].text, "no session");
        assert_eq!(entries[2].session_id, "unknown");
    }

    #[test]
    fn test_parse_command_history_empty_display() {
        let tmp = tempdir().unwrap();
        let history_path = tmp.path().join("history.jsonl");

        let content = r#"{"display":"","timestamp":1703001000000,"sessionId":"s1"}
{"display":"non-empty","timestamp":1703002000000,"sessionId":"s2"}"#;

        fs::write(&history_path, content).unwrap();

        let entries = parse_claude_command_history(&history_path).unwrap();
        // Empty display text is filtered out
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "non-empty");
    }

    #[test]
    fn test_parse_command_history_timestamp_conversion() {
        let tmp = tempdir().unwrap();
        let history_path = tmp.path().join("history.jsonl");

        // Timestamp is in milliseconds, should be converted to seconds
        let content = r#"{"display":"test","timestamp":1703001000123,"sessionId":"s1"}"#;

        fs::write(&history_path, content).unwrap();

        let entries = parse_claude_command_history(&history_path).unwrap();
        assert_eq!(entries[0].timestamp, 1703001000); // ms / 1000 = seconds
    }

    // =========================================================================
    // Session File Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_session_file_skill_tool() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("my-project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("session123.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"my-skill"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "my-skill");
        assert_eq!(events[0].session_id, "session123");
    }

    #[test]
    fn test_parse_session_file_read_skill_md() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read","input":{"file_path":"/home/user/skills/test/SKILL.md"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "/home/user/skills/test/SKILL.md");
    }

    #[test]
    fn test_parse_session_file_read_skills_directory() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read","input":{"file_path":"/path/to/skills/something.md"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].skill_path.contains("/skills/"));
    }

    #[test]
    fn test_parse_session_file_with_prompt_context() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T11:59:00Z","message":{"role":"user","content":[{"type":"text","text":"Please run the test skill"}]}}
{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"test-skill"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].prompt_context,
            Some("Please run the test skill".to_string())
        );
    }

    #[test]
    fn test_parse_session_file_skill_uri() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"uri":"skill://my-plugin/my-skill"}}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "skill://my-plugin/my-skill");
    }

    #[test]
    fn test_parse_session_file_empty() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("empty.jsonl");
        fs::write(&session_path, "").unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_session_file_malformed_entries() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess.jsonl");

        let content = r#"not valid json
{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"valid-skill"}}]}}
{"incomplete": true"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        // Should recover and parse valid entry
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "valid-skill");
    }

    #[test]
    fn test_parse_session_file_no_skill_events() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess.jsonl");

        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Read","input":{"file_path":"/some/regular/file.txt"}}]}}
{"timestamp":"2024-01-01T12:01:00Z","message":{"role":"assistant","content":[{"type":"text","text":"Just some text"}]}}"#;

        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_parse_session_file_multiple_projects() {
        let tmp = tempdir().unwrap();

        // Create two project directories
        let project1 = tmp.path().join("project1");
        let project2 = tmp.path().join("project2");
        fs::create_dir_all(&project1).unwrap();
        fs::create_dir_all(&project2).unwrap();

        // Add session files to each
        let content1 = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"skill-from-project1"}}]}}"#;
        let content2 = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"skill-from-project2"}}]}}"#;

        fs::write(project1.join("sess1.jsonl"), content1).unwrap();
        fs::write(project2.join("sess2.jsonl"), content2).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 2);

        let skill_paths: Vec<&str> = events.iter().map(|e| e.skill_path.as_str()).collect();
        assert!(skill_paths.contains(&"skill-from-project1"));
        assert!(skill_paths.contains(&"skill-from-project2"));
    }

    #[test]
    fn test_parse_session_ignores_non_jsonl_files() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();

        // Create various file types
        let jsonl_content = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"real-skill"}}]}}"#;
        fs::write(project_dir.join("session.jsonl"), jsonl_content).unwrap();
        fs::write(project_dir.join("notes.txt"), "just notes").unwrap();
        fs::write(project_dir.join("data.json"), "{}").unwrap();
        fs::write(project_dir.join("README.md"), "# Readme").unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "real-skill");
    }

    #[test]
    fn test_parse_session_ignores_non_directory_entries() {
        let tmp = tempdir().unwrap();

        // Create a file at the top level (not a project directory)
        fs::write(tmp.path().join("random-file.jsonl"), "{}").unwrap();

        // Create actual project directory
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let content = r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"skill"}}]}}"#;
        fs::write(project_dir.join("sess.jsonl"), content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
    }

    // =========================================================================
    // Timestamp Parsing Tests
    // =========================================================================

    #[test]
    fn test_parse_timestamp_valid_rfc3339() {
        let ts = parse_timestamp(Some("2024-01-15T10:30:00Z"));
        assert!(ts > 0);
        // 2024-01-15 10:30:00 UTC = 1705314600
        assert_eq!(ts, 1705314600);
    }

    #[test]
    fn test_parse_timestamp_with_timezone() {
        let ts = parse_timestamp(Some("2024-01-15T10:30:00+05:00"));
        assert!(ts > 0);
    }

    #[test]
    fn test_parse_timestamp_invalid() {
        assert_eq!(parse_timestamp(Some("not a date")), 0);
        assert_eq!(parse_timestamp(Some("")), 0);
        assert_eq!(parse_timestamp(None), 0);
    }

    // =========================================================================
    // Extract Skill Path Tests
    // =========================================================================

    #[test]
    fn test_extract_skill_path_from_skill_param() {
        let input = serde_json::json!({"skill": "my-skill-name"});
        assert_eq!(
            extract_skill_path(&input),
            Some("my-skill-name".to_string())
        );
    }

    #[test]
    fn test_extract_skill_path_from_uri() {
        let input = serde_json::json!({"uri": "skill://plugin/skill"});
        assert_eq!(
            extract_skill_path(&input),
            Some("skill://plugin/skill".to_string())
        );
    }

    #[test]
    fn test_extract_skill_path_from_uri_non_skill() {
        let input = serde_json::json!({"uri": "file:///some/path"});
        assert_eq!(extract_skill_path(&input), None);
    }

    #[test]
    fn test_extract_skill_path_from_file_path() {
        let input = serde_json::json!({"file_path": "/path/to/SKILL.md"});
        assert_eq!(
            extract_skill_path(&input),
            Some("/path/to/SKILL.md".to_string())
        );
    }

    #[test]
    fn test_extract_skill_path_from_file_path_non_skill() {
        let input = serde_json::json!({"file_path": "/path/to/README.md"});
        assert_eq!(extract_skill_path(&input), None);
    }

    #[test]
    fn test_extract_skill_path_empty_input() {
        let input = serde_json::json!({});
        assert_eq!(extract_skill_path(&input), None);
    }

    // =========================================================================
    // Prompt Context Truncation Tests
    // =========================================================================

    #[test]
    fn test_prompt_context_truncation() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess.jsonl");

        // Create a very long prompt
        let long_prompt = "a".repeat(500);
        let content = format!(
            r#"{{"timestamp":"2024-01-01T11:59:00Z","message":{{"role":"user","content":[{{"type":"text","text":"{}"}}]}}}}
{{"timestamp":"2024-01-01T12:00:00Z","message":{{"role":"assistant","content":[{{"type":"tool_use","name":"Skill","input":{{"skill":"test"}}}}]}}}}"#,
            long_prompt
        );

        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        // Prompt should be truncated to 200 chars
        assert_eq!(events[0].prompt_context.as_ref().unwrap().len(), 200);
    }
}
