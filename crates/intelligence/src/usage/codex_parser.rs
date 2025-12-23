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
#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
}
