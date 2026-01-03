//! Behavioral analysis of session tool sequences, file access patterns, and outcomes.
//!
//! This module extends the basic skill usage tracking with rich behavioral data:
//! - Tool call sequences within sessions (not just skill invocations)
//! - File access patterns (Read/Write/Edit operations)
//! - Session outcome detection (success/failure/partial)

use super::{SkillUsageEvent, UsageAnalytics};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
    pub confidence: f64,
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
pub fn extract_tool_calls(session_content: &str) -> Vec<ToolCall> {
    let mut tool_calls = Vec::new();

    for line in session_content.lines() {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
            // Look for tool_use in content blocks
            if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
                for block in content {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        if let Some(name) = block.get("name").and_then(|n| n.as_str()) {
                            let input_summary = block
                                .get("input")
                                .map(|i| safe_truncate(&i.to_string(), 200))
                                .unwrap_or_default();

                            let timestamp =
                                json.get("timestamp").and_then(|t| t.as_u64()).unwrap_or(0);

                            tool_calls.push(ToolCall {
                                name: name.to_string(),
                                timestamp,
                                input_summary,
                                status: ToolStatus::Unknown,
                            });
                        }
                    }
                }
            }

            // Look for tool_result to update status
            if let Some(content) = json.get("content").and_then(|c| c.as_array()) {
                for block in content {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        let is_error = block
                            .get("is_error")
                            .and_then(|e| e.as_bool())
                            .unwrap_or(false);

                        // Update most recent tool call's status
                        if let Some(last_call) = tool_calls.last_mut() {
                            if last_call.status == ToolStatus::Unknown {
                                last_call.status = if is_error {
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
                        }
                    }
                }
            }
        }
    }

    tool_calls
}

/// Extract file access events from tool calls.
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
            // Extract file path from input summary
            if let Some(path) = extract_file_path(&call.input_summary) {
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
pub fn detect_session_outcome(
    session_id: &str,
    tool_calls: &[ToolCall],
    prompt_contexts: &[Option<String>],
) -> SessionOutcome {
    if tool_calls.is_empty() {
        return SessionOutcome {
            session_id: session_id.to_string(),
            status: OutcomeStatus::Inconclusive,
            confidence: 0.0,
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
        (OutcomeStatus::Inconclusive, 0.3)
    } else {
        let success_ratio = success_signals as f64 / total_signals as f64;
        let confidence = (total_signals as f64 / 10.0).min(1.0);

        if success_ratio > 0.7 {
            (OutcomeStatus::Success, confidence)
        } else if success_ratio < 0.3 {
            (OutcomeStatus::Failure, confidence)
        } else {
            (OutcomeStatus::Partial, confidence * 0.8)
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

/// Detect retry patterns (consecutive calls to the same tool with similar inputs).
fn detect_retry_pattern(tool_calls: &[ToolCall]) -> usize {
    let mut max_consecutive = 0;
    let mut current_consecutive = 1;
    let mut prev_name: Option<&str> = None;

    for call in tool_calls {
        if Some(call.name.as_str()) == prev_name {
            current_consecutive += 1;
            max_consecutive = max_consecutive.max(current_consecutive);
        } else {
            current_consecutive = 1;
        }
        prev_name = Some(&call.name);
    }

    max_consecutive
}

// ============================================================================
// Pattern Aggregation
// ============================================================================

/// Build aggregated behavioral patterns from multiple sessions.
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

    common.sort_by(|a, b| b.1.cmp(&a.1));
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
            },
            ToolCall {
                name: "Read".to_string(),
                timestamp: 2000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Error {
                    message: "not found".to_string(),
                },
            },
            ToolCall {
                name: "Read".to_string(),
                timestamp: 3000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
            },
        ];

        assert_eq!(detect_retry_pattern(&calls), 3);
    }

    #[test]
    fn test_detect_session_outcome_empty() {
        let outcome = detect_session_outcome("test-session", &[], &[]);
        assert_eq!(outcome.status, OutcomeStatus::Inconclusive);
        assert_eq!(outcome.confidence, 0.0);
    }

    #[test]
    fn test_detect_session_outcome_success() {
        let calls = vec![
            ToolCall {
                name: "Read".to_string(),
                timestamp: 1000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
            },
            ToolCall {
                name: "Write".to_string(),
                timestamp: 2000,
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
            },
            ToolCall {
                name: "Bash".to_string(),
                timestamp: 3000 + 60, // 60+ seconds later
                input_summary: "{}".to_string(),
                status: ToolStatus::Success,
            },
        ];

        let contexts = vec![Some("Great, tests pass!".to_string())];
        let outcome = detect_session_outcome("test-session", &calls, &contexts);

        assert_eq!(outcome.status, OutcomeStatus::Success);
        assert!(outcome.confidence > 0.3);
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
            },
            ToolCall {
                name: "Bash".to_string(),
                timestamp: 1010,
                input_summary: "{}".to_string(),
                status: ToolStatus::Error {
                    message: "command failed again".to_string(),
                },
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
            },
            ToolCall {
                name: "Bash".to_string(),
                timestamp: 2000,
                input_summary: r#"{"command":"ls"}"#.to_string(),
                status: ToolStatus::Success,
            },
            ToolCall {
                name: "Write".to_string(),
                timestamp: 3000,
                input_summary: r#"{"file_path":"/src/lib.rs"}"#.to_string(),
                status: ToolStatus::Success,
            },
        ];

        let accesses = extract_file_accesses(&calls);

        assert_eq!(accesses.len(), 2);
        assert_eq!(accesses[0].path, "/src/main.rs");
        assert_eq!(accesses[0].operation, FileOperation::Read);
        assert_eq!(accesses[1].path, "/src/lib.rs");
        assert_eq!(accesses[1].operation, FileOperation::Write);
    }
}
