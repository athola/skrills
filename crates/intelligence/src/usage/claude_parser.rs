//! Parse Claude Code session files from ~/.claude/

use super::{CommandEntry, SkillUsageEvent};
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use tracing::{debug, warn};

/// Session files larger than this are skipped rather than scanned.
pub(super) const MAX_SESSION_FILE_BYTES: u64 = 256 * 1024 * 1024;
/// At most this many session files are scanned per build.
pub(super) const MAX_SESSION_FILES: usize = 20_000;
/// Command history files larger than this are skipped rather than read.
pub(super) const MAX_HISTORY_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Calls `on_line` with each non-blank line of the JSONL file at `path`.
///
/// Session logs can run to hundreds of megabytes, so the file is streamed,
/// and one invalid UTF-8 byte costs one line (decoded lossily), not the whole
/// file. A file larger than `max_bytes` is skipped with a warning, and so is
/// anything that is not a regular file: opening a FIFO blocks forever, and a
/// link to `/dev/zero` reports size 0 yet never ends. At most `max_bytes` are
/// read even if the file grows while it is read.
pub(super) fn for_each_session_line(
    path: &Path,
    max_bytes: u64,
    mut on_line: impl FnMut(&str),
) -> Result<()> {
    let meta = fs::metadata(path)?;
    if !meta.is_file() {
        warn!(path = %path.display(), "Skipping session path that is not a regular file");
        return Ok(());
    }
    let size = meta.len();
    if size > max_bytes {
        warn!(
            path = %path.display(),
            size,
            limit = max_bytes,
            "Skipping oversized session file"
        );
        return Ok(());
    }
    let mut reader = BufReader::new(fs::File::open(path)?.take(max_bytes));
    let mut buf = Vec::new();
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        let line = String::from_utf8_lossy(&buf);
        if !line.trim().is_empty() {
            on_line(&line);
        }
    }
    Ok(())
}

/// Parse Claude Code session files from ~/.claude/projects/
///
/// One unreadable project directory or session file is logged and skipped;
/// it does not fail the whole parse. Past `MAX_SESSION_FILES` the newest
/// files are kept.
pub fn parse_claude_sessions(projects_dir: &Path) -> Result<Vec<SkillUsageEvent>> {
    parse_claude_sessions_with_limit(projects_dir, MAX_SESSION_FILES)
}

pub(super) fn parse_claude_sessions_with_limit(
    projects_dir: &Path,
    max_files: usize,
) -> Result<Vec<SkillUsageEvent>> {
    let mut events = Vec::new();

    if !projects_dir.exists() {
        debug!(
            "Claude projects directory does not exist: {:?}",
            projects_dir
        );
        return Ok(events);
    }

    let mut sessions: Vec<(std::time::SystemTime, std::path::PathBuf)> = Vec::new();
    for project_entry in fs::read_dir(projects_dir)? {
        let project = match project_entry {
            Ok(p) => p,
            Err(e) => {
                warn!(dir = %projects_dir.display(), error = %e, "Skipping unreadable project entry");
                continue;
            }
        };
        let project_path = project.path();
        if !project.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }

        let entries = match fs::read_dir(&project_path) {
            Ok(s) => s,
            Err(e) => {
                warn!(dir = %project_path.display(), error = %e, "Skipping unreadable project directory");
                continue;
            }
        };
        for session_entry in entries {
            let Ok(session) = session_entry else {
                continue;
            };
            let path = session.path();
            if !path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                continue;
            }
            let mtime = session
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::UNIX_EPOCH);
            sessions.push((mtime, path));
        }
    }

    // Newest first, so a cap drops the oldest work rather than whatever the
    // directory listing happened to return last.
    sessions.sort_by_key(|s| std::cmp::Reverse(s.0));
    if sessions.len() > max_files {
        warn!(
            limit = max_files,
            found = sessions.len(),
            "Session file limit reached; oldest Claude sessions not analyzed"
        );
        sessions.truncate(max_files);
    }
    for (_, path) in sessions {
        match parse_claude_session_file(&path) {
            Ok(session_events) => events.extend(session_events),
            Err(e) => debug!("Failed to parse session file {:?}: {}", path, e),
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

    // Streamed like the session logs: one truncated multibyte paste costs one
    // line, not the whole history.
    for_each_session_line(history_path, MAX_HISTORY_FILE_BYTES, |line| {
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
    })?;

    Ok(entries)
}

fn parse_claude_session_file(path: &Path) -> Result<Vec<SkillUsageEvent>> {
    let mut events = Vec::new();

    let session_id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let mut last_user_prompt: Option<String> = None;

    for_each_session_line(path, MAX_SESSION_FILE_BYTES, |line| {
        let Ok(entry) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let Some(message) = entry.get("message") else {
            return;
        };

        // Track user prompts for context. Typed prompts are written as a
        // plain string; tool results and pasted blocks as an array.
        if message.get("role").and_then(|r| r.as_str()) == Some("user") {
            match message.get("content") {
                Some(Value::String(text)) => {
                    last_user_prompt = Some(text.chars().take(200).collect::<String>());
                }
                Some(Value::Array(contents)) => {
                    for content_block in contents {
                        if content_block.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(text) = content_block.get("text").and_then(|t| t.as_str()) {
                                last_user_prompt = Some(text.chars().take(200).collect::<String>());
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Look for tool_use content blocks
        let Some(contents) = message.get("content").and_then(|c| c.as_array()) else {
            return;
        };
        for content_block in contents {
            if content_block.get("type").and_then(|t| t.as_str()) != Some("tool_use") {
                continue;
            }
            let Some(name) = content_block.get("name").and_then(|n| n.as_str()) else {
                continue;
            };
            let Some(input) = content_block.get("input") else {
                continue;
            };
            let skill_path = if name.contains("skill") || name == "Skill" {
                // Skill-loading tools
                extract_skill_path(input)
            } else if name == "Read" {
                // Only a read of a SKILL.md loads a skill. Reading
                // `src/skills/mod.rs` or a skill's `references/x.md` does not.
                input
                    .get("file_path")
                    .and_then(|p| p.as_str())
                    .filter(|p| is_skill_md(p))
                    .map(str::to_string)
            } else {
                None
            };
            if let Some(skill_path) = skill_path {
                events.push(SkillUsageEvent {
                    timestamp: parse_timestamp(entry.get("timestamp").and_then(|t| t.as_str())),
                    skill_path,
                    session_id: session_id.clone(),
                    prompt_context: last_user_prompt.clone(),
                });
            }
        }
    })?;

    Ok(events)
}

/// True when `path` names a file called `SKILL.md`.
fn is_skill_md(path: &str) -> bool {
    path.rsplit(['/', '\\']).next() == Some("SKILL.md")
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

pub(super) fn parse_timestamp(s: Option<&str>) -> u64 {
    // A pre-1970 time would wrap to near u64::MAX ("used just now", forever),
    // so it reads as unknown (0) like an unparseable one.
    s.and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .and_then(|dt| u64::try_from(dt.timestamp()).ok())
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

    /// IN-34: reading other files under a `skills/` directory is not a skill
    /// load; only a read of a `SKILL.md` is.
    #[test]
    fn test_parse_session_file_read_skills_directory() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess.jsonl");

        let read = |p: &str| {
            format!(
                r#"{{"timestamp":"2024-01-01T12:00:00Z","message":{{"role":"assistant","content":[{{"type":"tool_use","name":"Read","input":{{"file_path":"{p}"}}}}]}}}}"#
            )
        };
        let content = [
            read("/path/to/skills/something.md"),
            read("/repo/src/skills/mod.rs"),
            read("/home/u/.claude/skills/foo/references/x.md"),
            read("/home/u/.claude/skills/foo/NOT-SKILL.md"),
            read("/home/u/.claude/skills/foo/SKILL.md"),
        ]
        .join("\n");
        fs::write(&session_path, content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        let paths: Vec<_> = events.iter().map(|e| e.skill_path.as_str()).collect();
        assert_eq!(paths, vec!["/home/u/.claude/skills/foo/SKILL.md"]);
    }

    /// IN-12: typed prompts are written with `content` as a plain string
    /// (checked against a real ~/.claude/projects session).
    #[test]
    fn prompt_context_is_captured_from_string_content() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let content = r#"{"timestamp":"2024-01-01T11:58:00Z","type":"user","message":{"role":"user","content":[{"type":"text","text":"an older prompt"}]}}
{"timestamp":"2024-01-01T11:59:00Z","type":"user","message":{"role":"user","content":"please review the code"}}
{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"review"}}]}}"#;
        fs::write(project_dir.join("sess.jsonl"), content).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].prompt_context.as_deref(),
            Some("please review the code")
        );
    }

    /// IN-33: one invalid UTF-8 byte costs its line, not the whole file.
    #[test]
    fn invalid_utf8_line_does_not_drop_the_session() {
        let tmp = tempdir().unwrap();
        let project_dir = tmp.path().join("project");
        fs::create_dir_all(&project_dir).unwrap();
        let mut bytes =
            b"{\"message\":{\"role\":\"user\",\"content\":\"bad \xff byte\"}}\n".to_vec();
        bytes.extend_from_slice(
            br#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"kept"}}]}}"#,
        );
        fs::write(project_dir.join("sess.jsonl"), bytes).unwrap();

        let events = parse_claude_sessions(tmp.path()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].skill_path, "kept");
    }

    /// IN-32: an unreadable project directory is skipped, not fatal.
    #[cfg(unix)]
    #[test]
    fn unreadable_project_directory_is_skipped() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir().unwrap();
        let good = tmp.path().join("good");
        let locked = tmp.path().join("locked");
        fs::create_dir_all(&good).unwrap();
        fs::create_dir_all(&locked).unwrap();
        fs::write(
            good.join("s.jsonl"),
            r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","name":"Skill","input":{"skill":"ok"}}]}}"#,
        )
        .unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();

        // Root ignores directory permissions; nothing to prove then.
        let locked_is_unreadable = fs::read_dir(&locked).is_err();
        let result = parse_claude_sessions(tmp.path());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        let events = result.expect("an unreadable project must not fail the parse");
        assert_eq!(events.len(), 1);
        if !locked_is_unreadable {
            eprintln!("running as root; unreadable-directory path not exercised");
        }
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

    #[test]
    fn timestamp_before_1970_is_unknown() {
        assert_eq!(parse_timestamp(Some("1960-01-01T00:00:00Z")), 0);
    }

    #[test]
    fn command_history_with_an_invalid_utf8_line_keeps_its_other_lines() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");
        let mut content = b"{\"display\":\"\xff broken\"}\n".to_vec();
        content.extend_from_slice(br#"{"display":"kept","timestamp":1000,"sessionId":"s"}"#);
        fs::write(&path, content).unwrap();
        let entries = parse_claude_command_history(&path).unwrap();
        assert!(entries.iter().any(|e| e.text == "kept"), "{entries:?}");
    }

    #[cfg(unix)]
    #[test]
    fn session_line_reader_skips_a_fifo() {
        let tmp = tempdir().unwrap();
        let fifo = tmp.path().join("pipe.jsonl");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        let mut lines = 0;
        for_each_session_line(&fifo, MAX_SESSION_FILE_BYTES, |_| lines += 1).unwrap();
        assert_eq!(lines, 0);
    }

    #[test]
    fn session_file_limit_keeps_the_newest_files() {
        let tmp = tempdir().unwrap();
        let project = tmp.path().join("p");
        fs::create_dir_all(&project).unwrap();
        let line = |skill: &str| {
            format!(
                r#"{{"timestamp":"2024-01-01T12:00:00Z","message":{{"role":"assistant","content":[{{"type":"tool_use","name":"Skill","input":{{"skill":"{skill}"}}}}]}}}}"#
            )
        };
        let now = std::time::SystemTime::now();
        for (i, name) in ["old", "newest", "middle"].iter().enumerate() {
            let path = project.join(format!("{name}.jsonl"));
            fs::write(&path, line(name)).unwrap();
            let age = match *name {
                "old" => 3000,
                "middle" => 2000,
                _ => 1000,
            };
            let _ = i;
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(now - std::time::Duration::from_secs(age))
                .unwrap();
        }
        let events = parse_claude_sessions_with_limit(tmp.path(), 2).unwrap();
        let mut skills: Vec<_> = events.iter().map(|e| e.skill_path.as_str()).collect();
        skills.sort();
        assert_eq!(skills, vec!["middle", "newest"]);
    }
}
