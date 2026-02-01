//! Data types for metrics.

use serde::{Deserialize, Serialize};

/// A recorded metric event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MetricEvent {
    /// Skill invocation event.
    SkillInvocation {
        /// Unique identifier.
        id: i64,
        /// Name of the skill.
        skill_name: String,
        /// Plugin name if applicable.
        plugin: Option<String>,
        /// Duration in milliseconds.
        duration_ms: u64,
        /// Whether the invocation succeeded.
        success: bool,
        /// Tokens used if tracked.
        tokens_used: Option<u64>,
        /// Timestamp of the event.
        created_at: String,
    },
    /// Validation run event.
    Validation {
        /// Unique identifier.
        id: i64,
        /// Name of the skill validated.
        skill_name: String,
        /// Checks that passed.
        checks_passed: Vec<String>,
        /// Checks that failed.
        checks_failed: Vec<String>,
        /// Timestamp of the event.
        created_at: String,
    },
    /// Sync operation event.
    Sync {
        /// Unique identifier.
        id: i64,
        /// Operation type (push/pull).
        operation: String,
        /// Number of files affected.
        files_count: usize,
        /// Status of the operation.
        status: String,
        /// Timestamp of the event.
        created_at: String,
    },
}

/// Statistics for a specific skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillStats {
    /// Total number of invocations.
    pub total_invocations: u64,
    /// Number of successful invocations.
    pub successful_invocations: u64,
    /// Number of failed invocations.
    pub failed_invocations: u64,
    /// Average duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Total tokens used across all invocations.
    pub total_tokens: u64,
}

/// A validation run record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRun {
    /// Unique identifier.
    pub id: i64,
    /// Name of the skill validated.
    pub skill_name: String,
    /// Checks that passed.
    pub checks_passed: Vec<String>,
    /// Checks that failed.
    pub checks_failed: Vec<String>,
    /// Timestamp of the run.
    pub created_at: String,
}

/// A sync event record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEvent {
    /// Unique identifier.
    pub id: i64,
    /// Operation type (push/pull).
    pub operation: String,
    /// Number of files affected.
    pub files_count: usize,
    /// Status of the operation.
    pub status: String,
    /// Timestamp of the event.
    pub created_at: String,
}
