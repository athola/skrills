//! Behavioral analysis of session tool sequences, file access patterns, and outcomes.
//!
//! This module extends the basic skill usage tracking with rich behavioral data:
//! - Tool call sequences within sessions (not just skill invocations)
//! - File access patterns (Read/Write/Edit operations)
//! - Session outcome detection (success/failure/partial)

use super::{SkillUsageEvent, UsageAnalytics};
use crate::types::Confidence;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tracing::trace;

// ============================================================================
// Helper Functions
// ============================================================================

/// Truncate a string safely at UTF-8 character boundaries.
fn safe_truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let mut end = max_len.saturating_sub(3); // Leave room for "..."
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

// ============================================================================
// Core Data Structures
// ============================================================================

/// Status of a tool execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolStatus {
    /// Tool executed successfully.
    Success,
    /// Tool execution resulted in an error.
    Error { message: String },
    /// Tool partially succeeded.
    Partial,
    /// Status could not be determined.
    #[default]
    Unknown,
}

/// A single tool call within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Tool name (e.g., "Read", "Write", "Bash", "Skill").
    pub name: String,
    /// Unix timestamp of the call.
    pub timestamp: u64,
    /// Truncated summary of input parameters (max 200 chars).
    pub input_summary: String,
    /// Execution status.
    pub status: ToolStatus,
    /// `file_path` from the full input, when the tool took one. Kept apart
    /// from `input_summary`, whose truncation usually cuts it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

/// Type of file operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileOperation {
    /// File was read.
    Read,
    /// File was written/created.
    Write,
    /// File was edited (partial modification).
    Edit {
        /// Optional line range affected.
        line_range: Option<(u64, u64)>,
    },
}

/// A file access event within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAccess {
    /// File path (relative when possible, truncated to 200 chars).
    pub path: String,
    /// Type of operation performed.
    pub operation: FileOperation,
    /// Unix timestamp of the access.
    pub timestamp: u64,
    /// Truncated prompt context that led to this access.
    pub context: Option<String>,
}

/// Status of a session outcome.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum OutcomeStatus {
    /// Task completed successfully (no errors, tests pass, etc.).
    Success,
    /// Task failed (errors, crashes, abandoned).
    Failure,
    /// Mixed results (some progress, some issues).
    Partial,
    /// Insufficient data to determine outcome.
    #[default]
    Inconclusive,
}

/// Analysis of a session's outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOutcome {
    /// Session identifier.
    pub session_id: String,
    /// Detected outcome status.
    pub status: OutcomeStatus,
    /// Confidence in the outcome detection (0.0 - 1.0).
    pub confidence: Confidence,
    /// Evidence supporting the outcome determination.
    pub evidence: Vec<String>,
    /// Session duration in seconds.
    pub duration_seconds: u64,
}

/// Enriched event combining skill usage with behavioral context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehavioralEvent {
    /// Original skill usage event.
    pub skill_usage: SkillUsageEventData,
    /// Sequence of tool calls in this session context.
    pub tool_sequence: Vec<ToolCall>,
    /// Files accessed during this session.
    pub files_accessed: Vec<FileAccess>,
    /// Session outcome if determinable.
    pub session_outcome: Option<SessionOutcome>,
}

/// Serializable version of SkillUsageEvent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillUsageEventData {
    pub timestamp: u64,
    pub skill_path: String,
    pub session_id: String,
    pub prompt_context: Option<String>,
}

impl From<&SkillUsageEvent> for SkillUsageEventData {
    fn from(event: &SkillUsageEvent) -> Self {
        Self {
            timestamp: event.timestamp,
            skill_path: event.skill_path.clone(),
            session_id: event.session_id.clone(),
            prompt_context: event.prompt_context.clone(),
        }
    }
}

// ============================================================================
// Aggregated Pattern Structures
// ============================================================================

/// Aggregated behavioral patterns across multiple sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BehavioralPatterns {
    /// Common tool sequences per skill (skill_path -> sequences).
    pub common_tool_sequences: HashMap<String, Vec<Vec<String>>>,
    /// File access patterns per skill (skill_path -> file patterns).
    pub file_access_patterns: HashMap<String, Vec<String>>,
    /// Indicators that correlate with successful sessions.
    pub success_indicators: Vec<String>,
    /// Indicators that correlate with failed sessions.
    pub failure_indicators: Vec<String>,
    /// Sessions analyzed for these patterns.
    pub sessions_analyzed: usize,
    /// Tool frequency across all sessions.
    pub tool_frequency: HashMap<String, u64>,
}

// ============================================================================
// Pattern Extraction Functions
// ============================================================================

/// Extract tool calls from raw session JSONL content.
///
/// Parses tool_use and tool_result blocks from Claude Code session format.
#[deprecated(
    since = "0.9.0",
    note = "the behavioural pipeline has no caller in skrills; unused, will be removed in a future release"
)]
pub fn extract_tool_calls(session_content: &str) -> Vec<ToolCall> {
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    // tool_use id -> index into `tool_calls`, so results match their call
    // even when several calls are issued in parallel.
    let mut index_by_id: HashMap<String, usize> = HashMap::new();

    for line in session_content.lines() {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // Claude Code nests blocks under `message.content` and writes an
        // RFC 3339 `timestamp`; older fixtures put `content` at the top level
        // with a numeric timestamp. Accept both.
        let Some(content) = json
            .get("message")
            .and_then(|m| m.get("content"))
            .or_else(|| json.get("content"))
            .and_then(|c| c.as_array())
        else {
            continue;
        };
        let timestamp = line_timestamp(&json);

        for block in content {
            match block.get("type").and_then(|t| t.as_str()) {
                Some("tool_use") => {
                    let Some(name) = block.get("name").and_then(|n| n.as_str()) else {
                        continue;
                    };
                    let input = block.get("input");
                    let input_summary = input
                        .map(|i| safe_truncate(&i.to_string(), 200))
                        .unwrap_or_default();
                    // Taken from the full input: the summary is truncated
                    // and usually loses it (Write/Edit carry file content).
                    let file_path = input
                        .and_then(|i| i.get("file_path"))
                        .and_then(|p| p.as_str())
                        .map(|p| safe_truncate(p, 200));

                    if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                        index_by_id.insert(id.to_string(), tool_calls.len());
                    }
                    tool_calls.push(ToolCall {
                        name: name.to_string(),
                        timestamp,
                        input_summary,
                        status: ToolStatus::Unknown,
                        file_path,
                    });
                }
                Some("tool_result") => {
                    let target = match block.get("tool_use_id").and_then(|i| i.as_str()) {
                        Some(id) => index_by_id.get(id).copied(),
                        // No id to match on: fall back to the latest call.
                        None => tool_calls.len().checked_sub(1),
                    };
                    let Some(call) = target.and_then(|i| tool_calls.get_mut(i)) else {
                        continue;
                    };
                    if call.status != ToolStatus::Unknown {
                        continue;
                    }
                    let is_error = block
                        .get("is_error")
                        .and_then(|e| e.as_bool())
                        .unwrap_or(false);
                    call.status = if is_error {
                        let error_msg = block
                            .get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or("Unknown error")
                            .chars()
                            .take(200)
                            .collect();
                        ToolStatus::Error { message: error_msg }
                    } else {
                        ToolStatus::Success
                    };
                }
                _ => {}
            }
        }
    }

    tool_calls
}

/// Unix seconds for a session line: RFC 3339 string (Claude Code) or number.
fn line_timestamp(json: &serde_json::Value) -> u64 {
    match json.get("timestamp") {
        Some(serde_json::Value::String(s)) => super::claude_parser::parse_timestamp(Some(s)),
        Some(v) => v.as_u64().unwrap_or(0),
        None => {
            trace!("timestamp missing in session line, defaulting to 0");
            0
        }
    }
}

/// Extract file access events from tool calls.
#[deprecated(
    since = "0.9.0",
    note = "the behavioural pipeline has no caller in skrills; unused, will be removed in a future release"
)]
pub fn extract_file_accesses(tool_calls: &[ToolCall]) -> Vec<FileAccess> {
    let mut accesses = Vec::new();

    for call in tool_calls {
        let operation = match call.name.as_str() {
            "Read" => Some(FileOperation::Read),
            "Write" => Some(FileOperation::Write),
            "Edit" => Some(FileOperation::Edit { line_range: None }),
            _ => None,
        };

        if let Some(op) = operation {
            let path = call
                .file_path
                .clone()
                .or_else(|| extract_file_path(&call.input_summary));
            if let Some(path) = path {
                accesses.push(FileAccess {
                    path,
                    operation: op,
                    timestamp: call.timestamp,
                    context: None,
                });
            }
        }
    }

    accesses
}

/// Extract file path from tool input summary.
fn extract_file_path(input_summary: &str) -> Option<String> {
    // Parse JSON input to find file_path parameter
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(input_summary) {
        if let Some(path) = json.get("file_path").and_then(|p| p.as_str()) {
            return Some(safe_truncate(path, 200));
        }
    }
    None
}

// ============================================================================
// Session Outcome Detection
// ============================================================================

/// Keywords indicating successful completion.
const SUCCESS_KEYWORDS: &[&str] = &[
    "tests pass",
    "all tests",
    "success",
    "completed",
    "done",
    "fixed",
    "resolved",
    "working",
    "build succeeded",
    "no errors",
];

/// Keywords indicating failure.
const FAILURE_KEYWORDS: &[&str] = &[
    "error",
    "failed",
    "failure",
    "crash",
    "exception",
    "traceback",
    "panic",
    "timeout",
    "rejected",
    "broken",
    "still broken",
];

/// Detect session outcome from tool calls and context.
#[deprecated(
    since = "0.9.0",
    note = "the behavioural pipeline has no caller in skrills; unused, will be removed in a future release"
)]
pub fn detect_session_outcome(
    session_id: &str,
    tool_calls: &[ToolCall],
    prompt_contexts: &[Option<String>],
) -> SessionOutcome {
    if tool_calls.is_empty() {
        trace!(
            session_id,
            "No tool calls in session, returning Inconclusive outcome"
        );
        return SessionOutcome {
            session_id: session_id.to_string(),
            status: OutcomeStatus::Inconclusive,
            confidence: Confidence::zero(),
            evidence: vec!["No tool calls in session".to_string()],
            duration_seconds: 0,
        };
    }

    let mut evidence = Vec::new();
    let mut success_signals = 0;
    let mut failure_signals = 0;

    // Analyze tool execution statuses
    let error_count = tool_calls
        .iter()
        .filter(|c| matches!(c.status, ToolStatus::Error { .. }))
        .count();
    let success_count = tool_calls
        .iter()
        .filter(|c| c.status == ToolStatus::Success)
        .count();

    if error_count > 0 {
        evidence.push(format!("{} tool errors detected", error_count));
        failure_signals += error_count;
    }
    if success_count > 0 {
        evidence.push(format!("{} successful tool calls", success_count));
        success_signals += 1;
    }

    // Check last few tool calls for indicators
    let recent_calls: Vec<_> = tool_calls.iter().rev().take(5).collect();
    let recent_errors = recent_calls
        .iter()
        .filter(|c| matches!(c.status, ToolStatus::Error { .. }))
        .count();

    if recent_errors == 0 && !recent_calls.is_empty() {
        evidence.push("No errors in final 5 tool calls".to_string());
        success_signals += 2;
    } else if recent_errors > 2 {
        evidence.push("Multiple errors in final tool calls".to_string());
        failure_signals += 2;
    }

    // Analyze prompt contexts for keywords
    for context in prompt_contexts.iter().flatten() {
        let lower = context.to_lowercase();
        for kw in SUCCESS_KEYWORDS {
            if lower.contains(kw) {
                success_signals += 1;
                evidence.push(format!("Success keyword detected: '{}'", kw));
            }
        }
        for kw in FAILURE_KEYWORDS {
            if lower.contains(kw) {
                failure_signals += 1;
                evidence.push(format!("Failure keyword detected: '{}'", kw));
            }
        }
    }

    // Check for retry patterns (same tool called repeatedly)
    let retry_count = detect_retry_pattern(tool_calls);
    if retry_count > 3 {
        evidence.push(format!(
            "Retry pattern detected: {} consecutive similar calls",
            retry_count
        ));
        failure_signals += retry_count / 2;
    }

    // Calculate duration
    let duration_seconds = if !tool_calls.is_empty() {
        let start = tool_calls.iter().map(|c| c.timestamp).min().unwrap_or(0);
        let end = tool_calls.iter().map(|c| c.timestamp).max().unwrap_or(0);
        end.saturating_sub(start)
    } else {
        0
    };

    // Duration heuristics
    if duration_seconds < 30 {
        evidence.push("Very short session (<30s)".to_string());
        failure_signals += 1; // May indicate abandoned session
    } else if duration_seconds > 60 {
        evidence.push(format!("Engaged session ({}s)", duration_seconds));
        success_signals += 1;
    }

    // Determine outcome
    let total_signals = success_signals + failure_signals;
    let (status, confidence) = if total_signals == 0 {
        (OutcomeStatus::Inconclusive, Confidence::new(0.3))
    } else {
        let success_ratio = success_signals as f64 / total_signals as f64;
        let conf_value = (total_signals as f64 / 10.0).min(1.0);

        if success_ratio > 0.7 {
            (OutcomeStatus::Success, Confidence::new(conf_value))
        } else if success_ratio < 0.3 {
            (OutcomeStatus::Failure, Confidence::new(conf_value))
        } else {
            (OutcomeStatus::Partial, Confidence::new(conf_value * 0.8))
        }
    };

    SessionOutcome {
        session_id: session_id.to_string(),
        status,
        confidence,
        evidence,
        duration_seconds,
    }
}

/// Detect retry patterns: the longest run of calls repeating the same tool
/// with the same input, each repeat following a failed attempt.
///
/// Ten `Read`s of different files are ordinary work, not retries.
fn detect_retry_pattern(tool_calls: &[ToolCall]) -> usize {
    let mut max_consecutive = 0;
    let mut current_consecutive = 1;
    let mut prev: Option<&ToolCall> = None;

    for call in tool_calls {
        let is_retry = prev.is_some_and(|p| {
            p.name == call.name
                && p.input_summary == call.input_summary
                && matches!(p.status, ToolStatus::Error { .. })
        });
        if is_retry {
            current_consecutive += 1;
            max_consecutive = max_consecutive.max(current_consecutive);
        } else {
            current_consecutive = 1;
        }
        prev = Some(call);
    }

    max_consecutive
}

// ============================================================================
// Pattern Aggregation
// ============================================================================

/// Build aggregated behavioral patterns from multiple sessions.
#[deprecated(
    since = "0.9.0",
    note = "the behavioural pipeline has no caller in skrills; unused, will be removed in a future release"
)]
pub fn build_behavioral_patterns(
    events: &[BehavioralEvent],
    analytics: &UsageAnalytics,
) -> BehavioralPatterns {
    let mut common_tool_sequences: HashMap<String, Vec<Vec<String>>> = HashMap::new();
    let mut file_access_patterns: HashMap<String, Vec<String>> = HashMap::new();
    let mut success_indicators: HashSet<String> = HashSet::new();
    let mut failure_indicators: HashSet<String> = HashSet::new();
    let mut tool_frequency: HashMap<String, u64> = HashMap::new();

    for event in events {
        let skill_path = &event.skill_usage.skill_path;

        // Collect tool sequences (first 10 tools after skill invocation)
        let sequence: Vec<String> = event
            .tool_sequence
            .iter()
            .take(10)
            .map(|t| t.name.clone())
            .collect();

        if !sequence.is_empty() {
            common_tool_sequences
                .entry(skill_path.clone())
                .or_default()
                .push(sequence);
        }

        // Collect file patterns
        for access in &event.files_accessed {
            file_access_patterns
                .entry(skill_path.clone())
                .or_default()
                .push(access.path.clone());
        }

        // Collect success/failure indicators
        if let Some(ref outcome) = event.session_outcome {
            match outcome.status {
                OutcomeStatus::Success => {
                    for ev in &outcome.evidence {
                        if ev.contains("Success keyword") || ev.contains("No errors") {
                            success_indicators.insert(ev.clone());
                        }
                    }
                }
                OutcomeStatus::Failure => {
                    for ev in &outcome.evidence {
                        if ev.contains("Failure keyword") || ev.contains("error") {
                            failure_indicators.insert(ev.clone());
                        }
                    }
                }
                _ => {}
            }
        }

        // Track tool frequency
        for call in &event.tool_sequence {
            *tool_frequency.entry(call.name.clone()).or_insert(0) += 1;
        }
    }

    // Deduplicate file patterns (keep unique per skill)
    for patterns in file_access_patterns.values_mut() {
        let unique: HashSet<_> = patterns.drain(..).collect();
        *patterns = unique.into_iter().collect();
    }

    // Get sessions count from analytics
    let sessions_analyzed = analytics.sessions_analyzed;

    BehavioralPatterns {
        common_tool_sequences,
        file_access_patterns,
        success_indicators: success_indicators.into_iter().collect(),
        failure_indicators: failure_indicators.into_iter().collect(),
        sessions_analyzed,
        tool_frequency,
    }
}

/// Extract common n-grams from tool sequences.
#[deprecated(
    since = "0.9.0",
    note = "the behavioural pipeline has no caller in skrills; unused, will be removed in a future release"
)]
pub fn extract_common_ngrams(
    sequences: &[Vec<String>],
    n: usize,
    min_count: usize,
) -> Vec<(Vec<String>, usize)> {
    let mut ngram_counts: HashMap<Vec<String>, usize> = HashMap::new();

    for sequence in sequences {
        if sequence.len() >= n {
            for window in sequence.windows(n) {
                *ngram_counts.entry(window.to_vec()).or_insert(0) += 1;
            }
        }
    }

    let mut common: Vec<_> = ngram_counts
        .into_iter()
        .filter(|(_, count)| *count >= min_count)
        .collect();

    common.sort_by_key(|b| std::cmp::Reverse(b.1));
    common
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_status_default() {
        assert_eq!(ToolStatus::default(), ToolStatus::Unknown);
    }

    #[test]
    fn test_outcome_status_default() {
        assert_eq!(OutcomeStatus::default(), OutcomeStatus::Inconclusive);
    }

    #[test]
    fn test_extract_file_path() {
        let input = r#"{"file_path":"/home/user/project/src/main.rs"}"#;
        let path = extract_file_path(input);
        assert_eq!(path, Some("/home/user/project/src/main.rs".to_string()));
    }

    #[test]
    fn test_extract_file_path_truncates_long_paths() {
        let long_path = "/".to_string() + &"a".repeat(300);
        let input = format!(r#"{{"file_path":"{}"}}"#, long_path);
        let path = extract_file_path(&input).unwrap();
        assert!(path.len() <= 200);
        assert!(path.ends_with("..."));
    }

    #[test]
    fn test_detect_retry_pattern() {
        let calls = vec![
            ToolCall {
                name: "Read".to_string(),
                timestamp: 1000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
                file_path: None,
            },
            ToolCall {
                name: "Read".to_string(),
                timestamp: 2000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Error {
                    message: "not found".to_string(),
                },
                file_path: None,
            },
            ToolCall {
                name: "Read".to_string(),
                timestamp: 3000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
                file_path: None,
            },
        ];

        // Only the call after the failure is a retry: a run of 2.
        assert_eq!(detect_retry_pattern(&calls), 2);
    }

    fn call(name: &str, input: &str, status: ToolStatus) -> ToolCall {
        ToolCall {
            name: name.to_string(),
            timestamp: 0,
            input_summary: input.to_string(),
            status,
            file_path: None,
        }
    }

    /// IN-37: the same tool on different inputs is not a retry, nor is a
    /// repeat that follows a success.
    #[test]
    fn retry_pattern_requires_same_input_after_an_error() {
        let reads: Vec<_> = (0..10)
            .map(|i| {
                call(
                    "Read",
                    &format!(r#"{{"file_path":"/f{i}"}}"#),
                    ToolStatus::Success,
                )
            })
            .collect();
        assert_eq!(detect_retry_pattern(&reads), 0);

        let err = || ToolStatus::Error {
            message: "boom".to_string(),
        };
        let retries = vec![
            call("Bash", "cargo test", err()),
            call("Bash", "cargo test", err()),
            call("Bash", "cargo test", err()),
            call("Bash", "cargo test", ToolStatus::Success),
        ];
        assert_eq!(detect_retry_pattern(&retries), 4);
    }

    /// IN-13: real Claude Code lines nest blocks under `message.content` and
    /// carry an RFC 3339 timestamp.
    #[test]
    fn extract_tool_calls_reads_claude_code_message_format() {
        let session = concat!(
            r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#,
            "\n",
            r#"{"timestamp":"2024-01-01T12:01:00Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#,
        );
        let calls = extract_tool_calls(session);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "Bash");
        assert_eq!(calls[0].timestamp, 1_704_110_400);
        assert_eq!(calls[0].status, ToolStatus::Success);
    }

    /// IN-36: results are matched to calls by `tool_use_id`.
    #[test]
    fn extract_tool_calls_matches_results_by_tool_use_id() {
        let session = concat!(
            r#"{"timestamp":"2024-01-01T12:00:00Z","message":{"role":"assistant","content":["#,
            r#"{"type":"tool_use","id":"a","name":"Read","input":{"file_path":"/a"}},"#,
            r#"{"type":"tool_use","id":"b","name":"Read","input":{"file_path":"/b"}},"#,
            r#"{"type":"tool_use","id":"c","name":"Read","input":{"file_path":"/c"}}]}}"#,
            "\n",
            r#"{"timestamp":"2024-01-01T12:00:01Z","message":{"role":"user","content":["#,
            r#"{"type":"tool_result","tool_use_id":"a","content":"ok"},"#,
            r#"{"type":"tool_result","tool_use_id":"b","is_error":true,"content":"missing"},"#,
            r#"{"type":"tool_result","tool_use_id":"c","content":"ok"}]}}"#,
        );
        let calls = extract_tool_calls(session);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].status, ToolStatus::Success);
        assert_eq!(
            calls[1].status,
            ToolStatus::Error {
                message: "missing".to_string()
            }
        );
        assert_eq!(calls[2].status, ToolStatus::Success);
    }

    /// IN-14: a Write whose input exceeds the 200-byte summary keeps its path.
    #[test]
    fn file_access_survives_long_tool_input() {
        let body = "x".repeat(5_000);
        let session = format!(
            r#"{{"timestamp":"2024-01-01T12:00:00Z","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"w","name":"Write","input":{{"content":"{body}","file_path":"/src/big.rs"}}}}]}}}}"#
        );
        let calls = extract_tool_calls(&session);
        assert!(calls[0].input_summary.len() <= 203);
        let accesses = extract_file_accesses(&calls);
        assert_eq!(accesses.len(), 1);
        assert_eq!(accesses[0].path, "/src/big.rs");
        assert_eq!(accesses[0].operation, FileOperation::Write);
    }

    #[test]
    fn test_detect_session_outcome_empty() {
        let outcome = detect_session_outcome("test-session", &[], &[]);
        assert_eq!(outcome.status, OutcomeStatus::Inconclusive);
        assert_eq!(outcome.confidence, Confidence::zero());
    }

    #[test]
    fn test_detect_session_outcome_success() {
        let calls = vec![
            ToolCall {
                name: "Read".to_string(),
                timestamp: 1000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
                file_path: None,
            },
            ToolCall {
                name: "Write".to_string(),
                timestamp: 2000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
                file_path: None,
            },
            ToolCall {
                name: "Bash".to_string(),
                timestamp: 3000 + 60, // 60+ seconds later
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
                file_path: None,
            },
        ];

        let contexts = vec![Some("Great, tests pass!".to_string())];
        let outcome = detect_session_outcome("test-session", &calls, &contexts);

        assert_eq!(outcome.status, OutcomeStatus::Success);
        assert!(outcome.confidence.value() > 0.3);
    }

    #[test]
    fn test_detect_session_outcome_failure() {
        let calls = vec![
            ToolCall {
                name: "Bash".to_string(),
                timestamp: 1000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Error {
                    message: "command failed".to_string(),
                },
                file_path: None,
            },
            ToolCall {
                name: "Bash".to_string(),
                timestamp: 1010,
                input_summary: "{}".to_string(),
                status: ToolStatus::Error {
                    message: "command failed again".to_string(),
                },
                file_path: None,
            },
        ];

        let contexts = vec![Some("Error: build failed".to_string())];
        let outcome = detect_session_outcome("test-session", &calls, &contexts);

        assert_eq!(outcome.status, OutcomeStatus::Failure);
    }

    #[test]
    fn test_extract_common_ngrams() {
        let sequences = vec![
            vec!["Read".to_string(), "Edit".to_string(), "Write".to_string()],
            vec!["Read".to_string(), "Edit".to_string(), "Write".to_string()],
            vec!["Read".to_string(), "Bash".to_string()],
        ];

        let bigrams = extract_common_ngrams(&sequences, 2, 2);

        // "Read", "Edit" should appear 2 times
        assert!(bigrams.iter().any(|(gram, count)| gram
            == &vec!["Read".to_string(), "Edit".to_string()]
            && *count == 2));
    }

    #[test]
    fn test_extract_file_accesses() {
        let calls = vec![
            ToolCall {
                name: "Read".to_string(),
                timestamp: 1000,
                input_summary: r#"{"file_path":"/src/main.rs"}"#.to_string(),
                status: ToolStatus::Success,
                file_path: None,
            },
            ToolCall {
                name: "Bash".to_string(),
                timestamp: 2000,
                input_summary: r#"{"command":"ls"}"#.to_string(),
                status: ToolStatus::Success,
                file_path: None,
            },
            ToolCall {
                name: "Write".to_string(),
                timestamp: 3000,
                input_summary: r#"{"file_path":"/src/lib.rs"}"#.to_string(),
                status: ToolStatus::Success,
                file_path: None,
            },
        ];

        let accesses = extract_file_accesses(&calls);

        assert_eq!(accesses.len(), 2);
        assert_eq!(accesses[0].path, "/src/main.rs");
        assert_eq!(accesses[0].operation, FileOperation::Read);
        assert_eq!(accesses[1].path, "/src/lib.rs");
        assert_eq!(accesses[1].operation, FileOperation::Write);
    }

    // ========================================================================
    // TC-4: N-gram edge cases
    // ========================================================================

    #[test]
    fn test_extract_common_ngrams_empty_sequences() {
        let sequences: Vec<Vec<String>> = vec![];
        let result = extract_common_ngrams(&sequences, 2, 1);
        assert!(result.is_empty(), "Empty input should produce empty output");
    }

    #[test]
    fn test_extract_common_ngrams_single_empty_sequence() {
        let sequences = vec![vec![]];
        let result = extract_common_ngrams(&sequences, 2, 1);
        assert!(
            result.is_empty(),
            "Empty sequence should produce no n-grams"
        );
    }

    #[test]
    fn test_extract_common_ngrams_n_greater_than_length() {
        let sequences = vec![vec!["Read".to_string(), "Write".to_string()]];
        // Request 3-grams from a 2-element sequence
        let result = extract_common_ngrams(&sequences, 3, 1);
        assert!(
            result.is_empty(),
            "n > sequence length should produce no n-grams"
        );
    }

    #[test]
    fn test_extract_common_ngrams_n_equals_length() {
        let sequences = vec![
            vec!["Read".to_string(), "Write".to_string()],
            vec!["Read".to_string(), "Write".to_string()],
        ];
        // Request 2-grams from 2-element sequences
        let result = extract_common_ngrams(&sequences, 2, 1);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0],
            (vec!["Read".to_string(), "Write".to_string()], 2)
        );
    }

    #[test]
    fn test_extract_common_ngrams_min_count_filters() {
        let sequences = vec![
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
            vec!["A".to_string(), "B".to_string(), "D".to_string()],
            vec!["X".to_string(), "Y".to_string(), "Z".to_string()],
        ];
        // min_count=2 should only return "A","B" which appears twice
        let result = extract_common_ngrams(&sequences, 2, 2);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(result[0].1, 2);
    }

    // ========================================================================
    // TC-6: build_behavioral_patterns tests
    // ========================================================================

    fn make_behavioral_event(
        skill_path: &str,
        session_id: &str,
        tools: Vec<&str>,
        files: Vec<(&str, FileOperation)>,
        outcome_status: OutcomeStatus,
        evidence: Vec<&str>,
    ) -> BehavioralEvent {
        BehavioralEvent {
            skill_usage: SkillUsageEventData {
                timestamp: 1000,
                skill_path: skill_path.to_string(),
                session_id: session_id.to_string(),
                prompt_context: Some("test context".to_string()),
            },
            tool_sequence: tools
                .into_iter()
                .enumerate()
                .map(|(i, name)| ToolCall {
                    name: name.to_string(),
                    timestamp: 1000 + i as u64 * 100,
                    input_summary: "{}".to_string(),
                    status: ToolStatus::Success,
                    file_path: None,
                })
                .collect(),
            files_accessed: files
                .into_iter()
                .map(|(path, op)| FileAccess {
                    path: path.to_string(),
                    operation: op,
                    timestamp: 1000,
                    context: None,
                })
                .collect(),
            session_outcome: Some(SessionOutcome {
                session_id: session_id.to_string(),
                status: outcome_status,
                confidence: Confidence::new(0.8),
                evidence: evidence.into_iter().map(|s| s.to_string()).collect(),
                duration_seconds: 60,
            }),
        }
    }

    #[test]
    fn test_build_behavioral_patterns_empty_events() {
        let analytics = UsageAnalytics {
            sessions_analyzed: 0,
            ..Default::default()
        };
        let patterns = build_behavioral_patterns(&[], &analytics);

        assert!(patterns.common_tool_sequences.is_empty());
        assert!(patterns.file_access_patterns.is_empty());
        assert!(patterns.success_indicators.is_empty());
        assert!(patterns.failure_indicators.is_empty());
        assert_eq!(patterns.sessions_analyzed, 0);
    }

    #[test]
    fn test_build_behavioral_patterns_collects_tool_sequences() {
        let events = vec![
            make_behavioral_event(
                "skill-a",
                "s1",
                vec!["Read", "Edit", "Write"],
                vec![],
                OutcomeStatus::Success,
                vec![],
            ),
            make_behavioral_event(
                "skill-a",
                "s2",
                vec!["Bash", "Read"],
                vec![],
                OutcomeStatus::Success,
                vec![],
            ),
        ];
        let analytics = UsageAnalytics {
            sessions_analyzed: 2,
            ..Default::default()
        };

        let patterns = build_behavioral_patterns(&events, &analytics);

        assert!(patterns.common_tool_sequences.contains_key("skill-a"));
        let sequences = &patterns.common_tool_sequences["skill-a"];
        assert_eq!(sequences.len(), 2);
    }

    #[test]
    fn test_build_behavioral_patterns_collects_file_patterns() {
        let events = vec![make_behavioral_event(
            "skill-b",
            "s1",
            vec!["Read"],
            vec![
                ("/src/main.rs", FileOperation::Read),
                ("/src/lib.rs", FileOperation::Write),
            ],
            OutcomeStatus::Success,
            vec![],
        )];
        let analytics = UsageAnalytics::default();

        let patterns = build_behavioral_patterns(&events, &analytics);

        assert!(patterns.file_access_patterns.contains_key("skill-b"));
        let files = &patterns.file_access_patterns["skill-b"];
        assert_eq!(files.len(), 2);
        assert!(files.contains(&"/src/main.rs".to_string()));
        assert!(files.contains(&"/src/lib.rs".to_string()));
    }

    #[test]
    fn test_build_behavioral_patterns_deduplicates_files() {
        let events = vec![
            make_behavioral_event(
                "skill-c",
                "s1",
                vec!["Read"],
                vec![("/src/main.rs", FileOperation::Read)],
                OutcomeStatus::Success,
                vec![],
            ),
            make_behavioral_event(
                "skill-c",
                "s2",
                vec!["Read"],
                vec![("/src/main.rs", FileOperation::Read)], // Same file
                OutcomeStatus::Success,
                vec![],
            ),
        ];
        let analytics = UsageAnalytics::default();

        let patterns = build_behavioral_patterns(&events, &analytics);

        let files = &patterns.file_access_patterns["skill-c"];
        // Should be deduplicated to 1
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn test_build_behavioral_patterns_collects_success_indicators() {
        let events = vec![make_behavioral_event(
            "skill-d",
            "s1",
            vec!["Bash"],
            vec![],
            OutcomeStatus::Success,
            vec!["Success keyword: tests pass", "No errors detected"],
        )];
        let analytics = UsageAnalytics::default();

        let patterns = build_behavioral_patterns(&events, &analytics);

        assert!(!patterns.success_indicators.is_empty());
        assert!(patterns
            .success_indicators
            .iter()
            .any(|s| s.contains("Success keyword")));
    }

    #[test]
    fn test_build_behavioral_patterns_collects_failure_indicators() {
        let events = vec![make_behavioral_event(
            "skill-e",
            "s1",
            vec!["Bash"],
            vec![],
            OutcomeStatus::Failure,
            vec!["Failure keyword: build failed", "error: compilation failed"],
        )];
        let analytics = UsageAnalytics::default();

        let patterns = build_behavioral_patterns(&events, &analytics);

        assert!(!patterns.failure_indicators.is_empty());
        assert!(patterns
            .failure_indicators
            .iter()
            .any(|s| s.contains("error")));
    }

    #[test]
    fn test_build_behavioral_patterns_tracks_tool_frequency() {
        let events = vec![
            make_behavioral_event(
                "skill-f",
                "s1",
                vec!["Read", "Read", "Write"],
                vec![],
                OutcomeStatus::Success,
                vec![],
            ),
            make_behavioral_event(
                "skill-f",
                "s2",
                vec!["Read", "Bash"],
                vec![],
                OutcomeStatus::Success,
                vec![],
            ),
        ];
        let analytics = UsageAnalytics::default();

        let patterns = build_behavioral_patterns(&events, &analytics);

        assert_eq!(patterns.tool_frequency.get("Read"), Some(&3));
        assert_eq!(patterns.tool_frequency.get("Write"), Some(&1));
        assert_eq!(patterns.tool_frequency.get("Bash"), Some(&1));
    }
}
