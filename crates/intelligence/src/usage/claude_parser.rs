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
}
