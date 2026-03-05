//! Data types for metrics.

use serde::{Deserialize, Serialize};

/// Sync operation type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncOperation {
    /// Push local changes to remote.
    Push,
    /// Pull remote changes to local.
    Pull,
}

impl SyncOperation {
    /// Returns the operation as a string slice.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Push => "push",
            Self::Pull => "pull",
        }
    }
}

impl std::fmt::Display for SyncOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Sync operation status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncStatus {
    /// Operation completed successfully.
    Success,
    /// Operation is in progress.
    #[serde(rename = "in_progress")]
    InProgress,
    /// Operation failed.
    Failed,
    /// Operation completed.
    Complete,
}

impl SyncStatus {
    /// Returns the status as a string slice.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Success => "success",
            Self::InProgress => "in_progress",
            Self::Failed => "failed",
            Self::Complete => "complete",
        }
    }
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse a sync operation from a database string.
pub(crate) fn parse_sync_operation(s: &str) -> SyncOperation {
    match s {
        "push" => SyncOperation::Push,
        _ => SyncOperation::Pull,
    }
}

/// Parse a sync status from a database string.
pub(crate) fn parse_sync_status(s: &str) -> SyncStatus {
    match s {
        "success" => SyncStatus::Success,
        "in_progress" => SyncStatus::InProgress,
        "failed" => SyncStatus::Failed,
        "complete" => SyncStatus::Complete,
        _ => SyncStatus::Success,
    }
}

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
        /// Operation type.
        operation: SyncOperation,
        /// Number of files affected.
        files_count: usize,
        /// Status of the operation.
        status: SyncStatus,
        /// Timestamp of the event.
        created_at: String,
    },
}

/// Statistics for a specific skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillStats {
    /// Number of successful invocations.
    pub successful_invocations: u64,
    /// Number of failed invocations.
    pub failed_invocations: u64,
    /// Average duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Total tokens used across all invocations.
    pub total_tokens: u64,
}

impl SkillStats {
    /// Total number of invocations (successful + failed).
    pub fn total_invocations(&self) -> u64 {
        self.successful_invocations + self.failed_invocations
    }
}

