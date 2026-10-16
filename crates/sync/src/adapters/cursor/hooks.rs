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
use std::fs;
use std::path::Path;
use std::time::SystemTime;
use tracing::{debug, warn};

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
///
/// Entries are kept as JSON values, as the writer handles them: Cursor entry
/// types without a `command` (a `prompt` hook) and fields this crate does not
/// name survive the trip. An entry that is not an object, or an event whose
/// value is not a list, is skipped with a warning instead of failing the phase.
pub fn read_hooks(root: &Path) -> Result<Vec<Command>> {
    let path = hooks_path(root);
    if !path.exists() {
        return Ok(vec![]);
    }

    let content = fs::read_to_string(&path)?;
    let config: serde_json::Value = serde_json::from_str(&content)?;
    let events = match config.get("hooks") {
        None => return Ok(vec![]),
        Some(serde_json::Value::Object(events)) => events,
        Some(_) => anyhow::bail!("`hooks` in {} is not an object", path.display()),
    };

    let mut hooks = Vec::new();
    let modified = fs::metadata(&path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);

    for (event_name, entries) in events {
        let Some(entries) = entries.as_array() else {
            warn!(event = %event_name, "Skipping hooks.json event whose value is not a list");
            continue;
        };
        let entries: Vec<&serde_json::Value> = entries
            .iter()
            .filter(|entry| {
                let keep = entry.is_object();
                if !keep {
                    warn!(event = %event_name, "Skipping hooks.json entry that is not an object");
                }
                keep
            })
            .collect();
        let entries_json = serde_json::to_string_pretty(&entries)?;
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

        // Both JSON and Markdown sources must hold a JSON array of entry
        // objects. Their fields are Cursor's to judge: a `prompt` hook has no
        // `command`.
        let content_str = String::from_utf8_lossy(&hook.content);
        let entries: Vec<serde_json::Value> = match serde_json::from_str::<
            Vec<serde_json::Map<String, serde_json::Value>>,
        >(&content_str)
        {
            Ok(entries) => entries.into_iter().map(serde_json::Value::Object).collect(),
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

/// Rewrites a plugin's `hooks/hooks.json` from Claude's layout
/// (`{"PostToolUse": [{"matcher", "hooks": [entry]}]}`) to Cursor's flat
/// camelCase one (`{"postToolUse": [entry + matcher]}`).
///
/// `${CLAUDE_PLUGIN_ROOT}` in a command becomes `plugin_root`, the mirror's
/// absolute path: Cursor documents plugin-root expansion for `mcp.json` only.
/// An event already in camelCase is taken as Cursor's and kept; a Claude event
/// with no Cursor equivalent is dropped and named in the returned warnings.
/// `Err` when the file is not a JSON object with a `hooks` object.
pub(super) fn plugin_hooks_for_cursor(
    content: &[u8],
    plugin_root: &Path,
) -> std::result::Result<(Vec<u8>, Vec<String>), String> {
    use serde_json::Value;

    let mut config: Value = serde_json::from_slice(content).map_err(|e| e.to_string())?;
    let root_obj = config
        .as_object_mut()
        .ok_or("the top level is not an object")?;
    let events = root_obj
        .get("hooks")
        .and_then(Value::as_object)
        .ok_or("it has no `hooks` object")?;

    // The root is pasted into a shell command unquoted (it often sits inside
    // the command's own quotes), so only a path with no shell syntax in it is
    // substituted; any other leaves the variable for the user to set.
    let root = plugin_root.to_str().filter(|root| {
        root.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-+@%:,~".contains(c))
    });
    let mut root_left_unset = false;
    let mut point_at_mirror = |entry: &mut Value| {
        if let Some(command) = entry.get_mut("command") {
            if let Some(text) = command.as_str() {
                if !text.contains("$CLAUDE_PLUGIN_ROOT") && !text.contains("${CLAUDE_PLUGIN_ROOT}")
                {
                    return;
                }
                match root {
                    Some(root) => *command = Value::from(replace_plugin_root(text, root)),
                    None => root_left_unset = true,
                }
            }
        }
    };

    let mut cursor_events = serde_json::Map::new();
    let mut dropped = Vec::new();
    for (event, groups) in events {
        let cursor_event = match claude_to_cursor_event(event) {
            Some(mapped) => mapped.to_string(),
            None if event.starts_with(|c: char| c.is_ascii_lowercase()) => event.clone(),
            None => {
                dropped.push(event.clone());
                continue;
            }
        };
        let groups = groups
            .as_array()
            .ok_or_else(|| format!("`hooks.{event}` is not an array"))?;
        let mut entries = Vec::new();
        for group in groups {
            match group.get("hooks").and_then(Value::as_array) {
                Some(nested) => {
                    let matcher = group.get("matcher").filter(|m| m.as_str() != Some(""));
                    for entry in nested {
                        let mut entry = entry.clone();
                        if let (Some(matcher), Some(fields)) = (matcher, entry.as_object_mut()) {
                            fields.entry("matcher").or_insert_with(|| matcher.clone());
                        }
                        point_at_mirror(&mut entry);
                        entries.push(entry);
                    }
                }
                None => {
                    let mut entry = group.clone();
                    point_at_mirror(&mut entry);
                    entries.push(entry);
                }
            }
        }
        cursor_events
            .entry(cursor_event)
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("inserted as an array")
            .extend(entries);
    }

    root_obj.insert("hooks".to_string(), Value::Object(cursor_events));
    root_obj.entry("version").or_insert_with(|| Value::from(1));

    let mut json = serde_json::to_vec_pretty(&config).map_err(|e| e.to_string())?;
    json.push(b'\n');
    let mut warnings = Vec::new();
    if !dropped.is_empty() {
        warnings.push(format!(
            "hooks.json events with no Cursor equivalent were dropped: {}",
            dropped.join(", ")
        ));
    }
    if root_left_unset {
        warnings.push(format!(
            "{} contains characters a shell would interpret, so ${{CLAUDE_PLUGIN_ROOT}} was \
             left in the hook commands; set it in Cursor's environment for them to run",
            plugin_root.display()
        ));
    }
    Ok((json, warnings))
}

/// `text` with `${CLAUDE_PLUGIN_ROOT}`, and `$CLAUDE_PLUGIN_ROOT` not followed
/// by another name character, replaced by `root`.
fn replace_plugin_root(text: &str, root: &str) -> String {
    const NAME: &str = "$CLAUDE_PLUGIN_ROOT";
    let text = text.replace("${CLAUDE_PLUGIN_ROOT}", root);
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find(NAME) {
        let after = &rest[at + NAME.len()..];
        out.push_str(&rest[..at]);
        if after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            out.push_str(NAME);
        } else {
            out.push_str(root);
        }
        rest = after;
    }
    out.push_str(rest);
    out
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

    /// A prompt-type entry has no `command`, and a fractional `timeout` is
    /// not a `u32`; either used to fail the whole hooks phase, and fields the
    /// struct did not name (`loop_limit`) were lost on the way through.
    #[test]
    fn read_hooks_keeps_entries_of_any_shape_and_skips_non_objects() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        let config = serde_json::json!({
            "version": 1,
            "hooks": {
                "stop": [
                    {"type": "prompt", "prompt": "Summarise the session"},
                    {"command": "./run.sh", "timeout": 1.5, "loop_limit": 3},
                    "not an entry"
                ]
            }
        });
        std::fs::write(hooks_path(root), serde_json::to_string(&config).unwrap()).unwrap();

        let hooks = read_hooks(root).unwrap();

        assert_eq!(hooks.len(), 1);
        let entries: Vec<serde_json::Value> = serde_json::from_slice(&hooks[0].content).unwrap();
        assert_eq!(
            entries,
            vec![
                serde_json::json!({"type": "prompt", "prompt": "Summarise the session"}),
                serde_json::json!({"command": "./run.sh", "timeout": 1.5, "loop_limit": 3}),
            ]
        );
    }

    #[test]
    fn read_hooks_empty_when_the_file_has_no_hooks_key() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(hooks_path(tmp.path()), r#"{"version": 1}"#).unwrap();

        let hooks = read_hooks(tmp.path()).unwrap();

        assert!(hooks.is_empty());
    }

    #[test]
    fn read_hooks_fails_naming_the_file_when_hooks_is_not_an_object() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = hooks_path(tmp.path());
        std::fs::write(&path, r#"{"hooks": ["stop"]}"#).unwrap();

        let err = read_hooks(tmp.path()).unwrap_err().to_string();

        assert!(err.contains("not an object"), "{err}");
        assert!(err.contains(&path.display().to_string()), "{err}");
    }

    #[test]
    fn read_hooks_skips_an_event_whose_value_is_not_a_list() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = serde_json::json!({
            "hooks": {
                "stop": {"command": "./not-in-a-list.sh"},
                "afterFileEdit": [{"command": "./fmt.sh"}]
            }
        });
        std::fs::write(
            hooks_path(tmp.path()),
            serde_json::to_string(&config).unwrap(),
        )
        .unwrap();

        let hooks = read_hooks(tmp.path()).unwrap();

        let names: Vec<&str> = hooks.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["afterFileEdit"]);
    }

    /// The same entries must survive being written back to Cursor.
    #[test]
    fn write_hooks_accepts_entries_without_a_command() {
        let tmp = tempfile::TempDir::new().unwrap();
        let hook = Command {
            name: "Stop".to_string(),
            content: br#"[{"type": "prompt", "prompt": "p"}, {"command": "x", "timeout": 1.5}]"#
                .to_vec(),
            source_path: tmp.path().join("hooks.json"),
            modified: SystemTime::UNIX_EPOCH,
            hash: String::new(),
            modules: vec![],
            content_format: ContentFormat::Json,
            plugin_origin: None,
        };

        let report = write_hooks(tmp.path(), &[hook]).unwrap();

        assert_eq!(report.written, 1, "skipped: {:?}", report.skipped);
    }

    fn translated_command(hooks_json: &str, root: &Path) -> (String, Vec<String>) {
        let (bytes, warnings) = plugin_hooks_for_cursor(hooks_json.as_bytes(), root).unwrap();
        let config: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let command = config["hooks"]["stop"][0]["command"]
            .as_str()
            .unwrap()
            .to_string();
        (command, warnings)
    }

    /// `$CLAUDE_PLUGIN_ROOT_DIR` is another variable; replacing its prefix
    /// produced `<root>_DIR`.
    #[test]
    fn plugin_root_is_replaced_only_as_a_whole_variable() {
        let (command, _) = translated_command(
            r#"{"hooks": {"Stop": [{"hooks": [{"command": "$CLAUDE_PLUGIN_ROOT/run.sh $CLAUDE_PLUGIN_ROOT_DIR"}]}]}}"#,
            Path::new("/home/u/.cursor/plugins/local/p"),
        );
        assert_eq!(
            command,
            "/home/u/.cursor/plugins/local/p/run.sh $CLAUDE_PLUGIN_ROOT_DIR"
        );
    }

    /// A root with shell syntax in it would change the command Cursor runs.
    #[test]
    fn a_plugin_root_with_shell_syntax_is_not_pasted_into_commands() {
        let (command, warnings) = translated_command(
            r#"{"hooks": {"Stop": [{"hooks": [{"command": "${CLAUDE_PLUGIN_ROOT}/run.sh"}]}]}}"#,
            Path::new("/home/u;rm -rf ~/.cursor/plugins/local/p"),
        );
        assert_eq!(command, "${CLAUDE_PLUGIN_ROOT}/run.sh");
        assert!(
            warnings.iter().any(|w| w.contains("CLAUDE_PLUGIN_ROOT")),
            "{warnings:?}"
        );
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
