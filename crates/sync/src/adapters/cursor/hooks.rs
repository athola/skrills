//! Hook reading and writing for Cursor adapter.
//!
//! Cursor hooks are configured in `.cursor/hooks.json` with camelCase event names
//! and 18+ lifecycle events. Claude Code uses PascalCase with 8 events.
//!
//! ## Event Mapping (Claude → Cursor)
//!
//! | Claude (PascalCase) | Cursor (camelCase)      |
//! |---------------------|-------------------------|
//! | PreToolUse          | preToolUse              |
//! | PostToolUse         | postToolUse             |
//! | SessionStart        | sessionStart            |
//! | SessionEnd          | sessionEnd              |
//! | Stop                | stop                    |
//! | SubagentStop        | subagentStop            |
//! | UserPromptSubmit    | beforeSubmitPrompt      |
//! | PreCompact          | preCompact              |
//! | Notification        | *(no equivalent)*       |

use super::paths::hooks_path;
use crate::adapters::utils::hash_content;
use crate::common::{Command, ContentFormat};
use crate::report::{SkipReason, WriteReport};
use crate::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::{debug, warn};

/// Cursor hooks.json top-level structure.
#[derive(Debug, Serialize, Deserialize, Default)]
struct CursorHooksConfig {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    hooks: BTreeMap<String, Vec<HookEntry>>,
}

/// A single hook entry within an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HookEntry {
    command: String,
    #[serde(default = "default_command_type")]
    r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matcher: Option<String>,
    #[serde(rename = "failClosed", skip_serializing_if = "Option::is_none")]
    fail_closed: Option<bool>,
}

fn default_command_type() -> String {
    "command".to_string()
}

/// Bidirectional event name mapping between Claude (PascalCase) and Cursor (camelCase).
static CLAUDE_TO_CURSOR_EVENTS: &[(&str, &str)] = &[
    ("PreToolUse", "preToolUse"),
    ("PostToolUse", "postToolUse"),
    ("SessionStart", "sessionStart"),
    ("SessionEnd", "sessionEnd"),
    ("Stop", "stop"),
    ("SubagentStop", "subagentStop"),
    ("UserPromptSubmit", "beforeSubmitPrompt"),
    ("PreCompact", "preCompact"),
];

/// Maps a Claude event name to Cursor event name.
pub fn claude_to_cursor_event(claude_event: &str) -> Option<&'static str> {
    CLAUDE_TO_CURSOR_EVENTS
        .iter()
        .find(|(c, _)| *c == claude_event)
        .map(|(_, cursor)| *cursor)
}

/// Maps a Cursor event name to Claude event name.
pub fn cursor_to_claude_event(cursor_event: &str) -> Option<&'static str> {
    CLAUDE_TO_CURSOR_EVENTS
        .iter()
        .find(|(_, c)| *c == cursor_event)
        .map(|(claude, _)| *claude)
}

/// Reads hooks from `.cursor/hooks.json`.
///
/// Each event group becomes a separate Command entry, with the event name
/// as the Command name and the JSON-serialized hook entries as content.
pub fn read_hooks(root: &Path) -> Result<Vec<Command>> {
    let path = hooks_path(root);
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&path)?;
    let config: CursorHooksConfig = serde_json::from_str(&content)?;

    let mut hooks = Vec::new();
    let modified = fs::metadata(&path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    for (event_name, entries) in &config.hooks {
        let entries_json = serde_json::to_string_pretty(entries)?;
        let content_bytes = entries_json.into_bytes();
        let hash = hash_content(&content_bytes);

        let name = cursor_to_claude_event(event_name)
            .map(|s| s.to_string())
            .unwrap_or_else(|| event_name.clone());

        hooks.push(Command {
            name,
            content: content_bytes,
            source_path: path.clone(),
            modified,
            hash,
            modules: vec![],
            content_format: ContentFormat::Json,
        });
    }

    hooks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(hooks)
}

/// Writes hooks to `.cursor/hooks.json`.
///
/// Translates Claude PascalCase event names to Cursor camelCase.
/// Events without a Cursor equivalent (e.g., Notification) are skipped.
pub fn write_hooks(root: &Path, hooks: &[Command]) -> Result<WriteReport> {
    let mut report = WriteReport::default();

    if hooks.is_empty() {
        return Ok(report);
    }

    // Read existing hooks to preserve Cursor-only events
    let path = hooks_path(root);
    let mut config = if path.exists() {
        let content = fs::read_to_string(&path)?;
        match serde_json::from_str::<CursorHooksConfig>(&content) {
            Ok(config) => config,
            Err(e) => {
                warn!(
                    path = %path.display(),
                    error = %e,
                    "Existing hooks.json has invalid JSON; starting fresh"
                );
                report
                    .warnings
                    .push(format!("Existing hooks.json could not be parsed: {}", e));
                CursorHooksConfig::default()
            }
        }
    } else {
        CursorHooksConfig::default()
    };
    config.version = 1;

    for hook in hooks {
        // Try to map the event name
        let cursor_event = if let Some(mapped) = claude_to_cursor_event(&hook.name) {
            mapped.to_string()
        } else {
            // Check if it's already a Cursor event name (passthrough)
            if CLAUDE_TO_CURSOR_EVENTS.iter().any(|(_, c)| *c == hook.name) {
                hook.name.clone()
            } else {
                warn!(
                    event = %hook.name,
                    "Skipping hook with no Cursor equivalent"
                );
                report.skipped.push(SkipReason::AgentSpecificFeature {
                    item: hook.name.clone(),
                    feature: format!("Hook event '{}' has no Cursor equivalent", hook.name),
                    suggestion: "This hook event is Claude-specific and cannot be mapped to Cursor"
                        .to_string(),
                });
                continue;
            }
        };

        // Parse hook content based on its declared format
        let content_str = String::from_utf8_lossy(&hook.content);
        let entries: Vec<HookEntry> = if hook.content_format == ContentFormat::Json {
            // JSON source — attempt parse, skip gracefully on failure
            match serde_json::from_str(&content_str) {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        event = %hook.name,
                        error = %e,
                        "Skipping hook with malformed JSON content"
                    );
                    report.skipped.push(SkipReason::AgentSpecificFeature {
                        item: hook.name.clone(),
                        feature: "Hook content is not valid JSON".to_string(),
                        suggestion: "Hook content must be a JSON array of hook entries".to_string(),
                    });
                    continue;
                }
            }
        } else {
            // Markdown/plain text — try JSON first, skip if unparseable
            match serde_json::from_str(&content_str) {
                Ok(e) => e,
                Err(e) => {
                    warn!(
                        event = %hook.name,
                        error = %e,
                        "Skipping hook with non-JSON content"
                    );
                    report.skipped.push(SkipReason::AgentSpecificFeature {
                        item: hook.name.clone(),
                        feature: "Hook content is not valid JSON".to_string(),
                        suggestion: "Hook content must be a JSON array of hook entries".to_string(),
                    });
                    continue;
                }
            }
        };

        if entries.is_empty() {
            debug!(event = %cursor_event, "Skipping hook with empty entry list");
            continue;
        }

        debug!(event = %cursor_event, count = entries.len(), "Writing Cursor hook");
        config.hooks.insert(cursor_event, entries);
        report.written += 1;
    }

    if !config.hooks.is_empty() {
        let path = hooks_path(root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&config)?;
        fs::write(&path, json)?;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_mapping_claude_to_cursor() {
        assert_eq!(claude_to_cursor_event("PreToolUse"), Some("preToolUse"));
        assert_eq!(claude_to_cursor_event("PostToolUse"), Some("postToolUse"));
        assert_eq!(
            claude_to_cursor_event("UserPromptSubmit"),
            Some("beforeSubmitPrompt")
        );
        assert_eq!(claude_to_cursor_event("Notification"), None);
        assert_eq!(claude_to_cursor_event("UnknownEvent"), None);
    }

    #[test]
    fn event_mapping_cursor_to_claude() {
        assert_eq!(cursor_to_claude_event("preToolUse"), Some("PreToolUse"));
        assert_eq!(
            cursor_to_claude_event("beforeSubmitPrompt"),
            Some("UserPromptSubmit")
        );
        assert_eq!(cursor_to_claude_event("beforeShellExecution"), None); // Cursor-only
        assert_eq!(cursor_to_claude_event("afterFileEdit"), None); // Cursor-only
    }

    #[test]
    fn event_mapping_bidirectional_roundtrip() {
        for (claude, cursor) in CLAUDE_TO_CURSOR_EVENTS {
            assert_eq!(claude_to_cursor_event(claude), Some(*cursor));
            assert_eq!(cursor_to_claude_event(cursor), Some(*claude));
        }
    }

    #[test]
    fn write_hooks_skips_empty_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let hooks = vec![crate::common::Command {
            name: "PreToolUse".to_string(),
            content: b"[]".to_vec(),
            source_path: std::path::PathBuf::from("/test"),
            modified: std::time::SystemTime::UNIX_EPOCH,
            hash: "test".to_string(),
            modules: vec![],
            content_format: ContentFormat::Json,
        }];

        let report = write_hooks(root, &hooks).unwrap();
        // Empty entry list is skipped, not written
        assert_eq!(report.written, 0);
    }

    #[test]
    fn write_hooks_skips_non_json_markdown_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let hooks = vec![crate::common::Command {
            name: "PreToolUse".to_string(),
            content: b"# This is markdown, not JSON".to_vec(),
            source_path: std::path::PathBuf::from("/test"),
            modified: std::time::SystemTime::UNIX_EPOCH,
            hash: "test".to_string(),
            modules: vec![],
            content_format: ContentFormat::default(), // Markdown
        }];

        let report = write_hooks(root, &hooks).unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn write_hooks_passthrough_cursor_event_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        // Pass a Cursor camelCase event name directly (already mapped)
        let hooks = vec![crate::common::Command {
            name: "preToolUse".to_string(),
            content: br#"[{"command": "./lint.sh", "type": "command"}]"#.to_vec(),
            source_path: std::path::PathBuf::from("/test"),
            modified: std::time::SystemTime::UNIX_EPOCH,
            hash: "test".to_string(),
            modules: vec![],
            content_format: ContentFormat::Json,
        }];

        let report = write_hooks(root, &hooks).unwrap();
        assert_eq!(report.written, 1);

        let content = std::fs::read_to_string(hooks_path(root)).unwrap();
        assert!(content.contains("preToolUse"));
    }

    #[test]
    fn read_hooks_empty_when_no_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hooks = read_hooks(tmp.path()).unwrap();
        assert!(hooks.is_empty());
    }

    #[test]
    fn read_hooks_preserves_cursor_only_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let config = serde_json::json!({
            "version": 1,
            "hooks": {
                "afterFileEdit": [{"command": "./format.sh", "type": "command"}],
                "preToolUse": [{"command": "./lint.sh", "type": "command"}]
            }
        });
        std::fs::write(hooks_path(root), serde_json::to_string(&config).unwrap()).unwrap();

        let hooks = read_hooks(root).unwrap();
        assert_eq!(hooks.len(), 2);

        let names: Vec<&str> = hooks.iter().map(|h| h.name.as_str()).collect();
        // preToolUse maps to PreToolUse
        assert!(names.contains(&"PreToolUse"));
        // afterFileEdit has no Claude equivalent, preserved as-is
        assert!(names.contains(&"afterFileEdit"));
    }

    #[test]
    fn hooks_write_read_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        // Write hooks with Claude PascalCase names
        let hooks = vec![
            crate::common::Command {
                name: "PreToolUse".to_string(),
                content: br#"[{"command": "./lint.sh", "type": "command"}]"#.to_vec(),
                source_path: std::path::PathBuf::from("/test"),
                modified: std::time::SystemTime::UNIX_EPOCH,
                hash: "test".to_string(),
                modules: vec![],
                content_format: ContentFormat::Json,
            },
            crate::common::Command {
                name: "PostToolUse".to_string(),
                content: br#"[{"command": "./format.sh", "type": "command"}]"#.to_vec(),
                source_path: std::path::PathBuf::from("/test"),
                modified: std::time::SystemTime::UNIX_EPOCH,
                hash: "test".to_string(),
                modules: vec![],
                content_format: ContentFormat::Json,
            },
        ];

        let write_report = write_hooks(root, &hooks).unwrap();
        assert_eq!(write_report.written, 2);

        // Read back — should get Claude PascalCase names
        let read_back = read_hooks(root).unwrap();
        let names: Vec<&str> = read_back.iter().map(|h| h.name.as_str()).collect();
        assert!(
            names.contains(&"PreToolUse"),
            "PreToolUse should survive roundtrip"
        );
        assert!(
            names.contains(&"PostToolUse"),
            "PostToolUse should survive roundtrip"
        );
    }

    #[test]
    fn write_hooks_gracefully_skips_malformed_json_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        let hooks = vec![crate::common::Command {
            name: "PreToolUse".to_string(),
            content: b"{not valid json}".to_vec(),
            source_path: std::path::PathBuf::from("/test"),
            modified: std::time::SystemTime::UNIX_EPOCH,
            hash: "test".to_string(),
            modules: vec![],
            content_format: ContentFormat::Json,
        }];

        // Should NOT error — should gracefully skip
        let report = write_hooks(root, &hooks).unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.skipped.len(), 1);
    }

    #[test]
    fn write_hooks_warns_on_malformed_existing_hooks_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();

        // Write malformed hooks.json
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(hooks_path(root), "{ this is not valid json }").unwrap();

        let hooks = vec![crate::common::Command {
            name: "PreToolUse".to_string(),
            content: br#"[{"command": "./lint.sh", "type": "command"}]"#.to_vec(),
            source_path: std::path::PathBuf::from("/test"),
            modified: std::time::SystemTime::UNIX_EPOCH,
            hash: "test".to_string(),
            modules: vec![],
            content_format: ContentFormat::Json,
        }];

        // Should NOT error — should warn and proceed with fresh config
        let report = write_hooks(root, &hooks).unwrap();
        assert_eq!(report.written, 1);
        assert!(
            !report.warnings.is_empty(),
            "Should have a warning about malformed JSON"
        );
    }
}
