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
            plugin_origin: None,
        });
    }

    hooks.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(hooks)
}

/// Writes hooks to `.cursor/hooks.json`.
///
/// Translates Claude PascalCase event names to Cursor camelCase.
/// Events without a Cursor equivalent (e.g., Notification) are skipped.
///
/// The file is edited as a JSON value: unknown top-level keys and unknown
/// per-entry fields survive, and a synced entry is merged into its event's
/// list (replacing an entry with the same `command` and `matcher`) instead of
/// replacing the list. A `hooks.json` that does not parse is left untouched and
/// every hook is reported skipped: it used to be replaced by the synced entries
/// alone.
pub fn write_hooks(root: &Path, hooks: &[Command]) -> Result<WriteReport> {
    let mut report = WriteReport::default();

    if hooks.is_empty() {
        return Ok(report);
    }

    let path = hooks_path(root);
    let mut config = match crate::adapters::json_config::load_object(&path) {
        Ok(config) => config,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "Leaving unreadable hooks.json untouched");
            report.warnings.push(format!(
                "{} could not be parsed, so no hooks were synced into it: {e}",
                path.display()
            ));
            for hook in hooks {
                report.skipped.push(SkipReason::ParseError {
                    item: hook.name.clone(),
                    error: format!("target {} is not valid JSON", path.display()),
                });
            }
            return Ok(report);
        }
    };

    let root_obj = config
        .as_object_mut()
        .expect("load_object returns an object");
    root_obj
        .entry("version")
        .or_insert_with(|| serde_json::Value::from(1));
    let events = root_obj
        .entry("hooks")
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    let Some(events) = events.as_object_mut() else {
        report.warnings.push(format!(
            "`hooks` in {} is not an object, so no hooks were synced into it",
            path.display()
        ));
        return Ok(report);
    };

    for hook in hooks {
        let cursor_event = if let Some(mapped) = claude_to_cursor_event(&hook.name) {
            mapped.to_string()
        } else if CLAUDE_TO_CURSOR_EVENTS.iter().any(|(_, c)| *c == hook.name) {
            // Already a Cursor event name (passthrough).
            hook.name.clone()
        } else {
            warn!(event = %hook.name, "Skipping hook with no Cursor equivalent");
            report.skipped.push(SkipReason::AgentSpecificFeature {
                item: hook.name.clone(),
                feature: format!("Hook event '{}' has no Cursor equivalent", hook.name),
                suggestion: "This hook event is Claude-specific and cannot be mapped to Cursor"
                    .to_string(),
            });
            continue;
        };

        // Both JSON and Markdown sources must hold a JSON array of entries.
        let content_str = String::from_utf8_lossy(&hook.content);
        let entries: Vec<serde_json::Value> =
            match serde_json::from_str::<Vec<HookEntry>>(&content_str)
                .and_then(|_| serde_json::from_str(&content_str))
            {
                Ok(entries) => entries,
                Err(e) => {
                    warn!(event = %hook.name, error = %e, "Skipping hook with non-JSON content");
                    report.skipped.push(SkipReason::ParseError {
                        item: hook.name.clone(),
                        error: format!("hook content must be a JSON array of hook entries: {e}"),
                    });
                    continue;
                }
            };

        if entries.is_empty() {
            debug!(event = %cursor_event, "Skipping hook with empty entry list");
            continue;
        }

        let list = events
            .entry(cursor_event.clone())
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        let Some(list) = list.as_array_mut() else {
            report.skipped.push(SkipReason::ParseError {
                item: hook.name.clone(),
                error: format!("`hooks.{cursor_event}` in hooks.json is not an array"),
            });
            continue;
        };
        let before = list.clone();
        for entry in entries {
            let key = entry_identity(&entry);
            match list.iter_mut().find(|e| entry_identity(e) == key) {
                Some(existing) => *existing = entry,
                None => list.push(entry),
            }
        }
        if *list == before {
            report.skipped.push(SkipReason::Unchanged {
                item: format!("hooks.{cursor_event}"),
            });
        } else {
            debug!(event = %cursor_event, "Writing Cursor hook");
            report.written += 1;
        }
    }

    if report.written > 0 {
        let json = serde_json::to_string_pretty(&config)?;
        crate::adapters::utils::write_config(&path, json.as_bytes(), false)?;
    }

    Ok(report)
}

/// What makes two hook entries the same hook: the command it runs and the
/// tool matcher it runs for.
fn entry_identity(entry: &serde_json::Value) -> (Option<&str>, Option<&str>) {
    (
        entry.get("command").and_then(|v| v.as_str()),
        entry.get("matcher").and_then(|v| v.as_str()),
    )
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
            plugin_origin: None,
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
            plugin_origin: None,
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
            plugin_origin: None,
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
                plugin_origin: None,
            },
            crate::common::Command {
                name: "PostToolUse".to_string(),
                content: br#"[{"command": "./format.sh", "type": "command"}]"#.to_vec(),
                source_path: std::path::PathBuf::from("/test"),
                modified: std::time::SystemTime::UNIX_EPOCH,
                hash: "test".to_string(),
                modules: vec![],
                content_format: ContentFormat::Json,
                plugin_origin: None,
            },
        ];

        let write_report = write_hooks(root, &hooks).unwrap();
        assert_eq!(write_report.written, 2);

        // Read back, should get Claude PascalCase names
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
            plugin_origin: None,
        }];

        // Should NOT error, should gracefully skip
        let report = write_hooks(root, &hooks).unwrap();
        assert_eq!(report.written, 0);
        assert_eq!(report.skipped.len(), 1);
    }

    fn json_hook(event: &str, entries: &str) -> crate::common::Command {
        crate::common::Command {
            name: event.to_string(),
            content: entries.as_bytes().to_vec(),
            source_path: std::path::PathBuf::from("/test"),
            modified: std::time::SystemTime::UNIX_EPOCH,
            hash: "test".to_string(),
            modules: vec![],
            content_format: ContentFormat::Json,
            plugin_origin: None,
        }
    }

    /// A hooks.json with a trailing comma used to be replaced by the synced
    /// entries alone, losing every hook the user wrote.
    #[test]
    fn write_hooks_leaves_an_unparseable_hooks_json_untouched() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let original = r#"{"hooks": {"preToolUse": [{"command": "./mine.sh"}],}}"#;
        std::fs::write(hooks_path(root), original).unwrap();

        let report = write_hooks(
            root,
            &[json_hook("PreToolUse", r#"[{"command": "./lint.sh"}]"#)],
        )
        .unwrap();

        assert_eq!(report.written, 0);
        assert!(!report.warnings.is_empty());
        assert!(matches!(report.skipped[0], SkipReason::ParseError { .. }));
        assert_eq!(std::fs::read_to_string(hooks_path(root)).unwrap(), original);
    }

    /// Existing entries for a mapped event were replaced, and unknown keys
    /// dropped by the typed structs.
    #[test]
    fn write_hooks_merges_entries_and_keeps_unknown_fields() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::write(
            hooks_path(root),
            r#"{"version": 1, "experimental": true, "hooks": {"preToolUse": [{"command": "./mine.sh", "env": {"A": "1"}}]}}"#,
        )
        .unwrap();

        let report = write_hooks(
            root,
            &[json_hook("PreToolUse", r#"[{"command": "./lint.sh"}]"#)],
        )
        .unwrap();
        assert_eq!(report.written, 1);

        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(hooks_path(root)).unwrap()).unwrap();
        assert_eq!(config["experimental"], true);
        let entries = config["hooks"]["preToolUse"].as_array().unwrap();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0]["command"], "./mine.sh");
        assert_eq!(entries[0]["env"]["A"], "1");
        assert_eq!(entries[1]["command"], "./lint.sh");

        let again = write_hooks(
            root,
            &[json_hook("PreToolUse", r#"[{"command": "./lint.sh"}]"#)],
        )
        .unwrap();
        assert_eq!(
            again.written, 0,
            "a repeat sync must not duplicate the entry"
        );
    }
}
