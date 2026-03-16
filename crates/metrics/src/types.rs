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
}

impl SyncStatus {
    /// Returns the status as a string slice.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Success => "success",
            Self::InProgress => "in_progress",
            Self::Failed => "failed",
        }
    }
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse a sync operation from a database string.
///
/// Logs a warning and defaults to `Pull` for unrecognized values.
pub(crate) fn parse_sync_operation(s: &str) -> SyncOperation {
    match s {
        "push" => SyncOperation::Push,
        "pull" => SyncOperation::Pull,
        other => {
            tracing::warn!(value = other, "Unknown sync operation in DB, defaulting to Pull");
            SyncOperation::Pull
        }
    }
}

/// Parse a sync status from a database string.
///
/// Logs a warning and defaults to `Failed` for unrecognized values,
/// avoiding silent inflation of success counts.
pub(crate) fn parse_sync_status(s: &str) -> SyncStatus {
    match s {
        "success" | "complete" => SyncStatus::Success,
        "in_progress" => SyncStatus::InProgress,
        "failed" => SyncStatus::Failed,
        other => {
            tracing::warn!(value = other, "Unknown sync status in DB, defaulting to Failed");
            SyncStatus::Failed
        }
    }
}

/// Outcome of a rule trigger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleOutcome {
    /// Rule check passed.
    Pass,
    /// Rule check failed (violation detected).
    Fail,
    /// Rule was skipped.
    Skip,
    /// Rule encountered an error.
    Error,
}

impl RuleOutcome {
    /// Returns the outcome as a string slice.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pass => "pass",
            Self::Fail => "fail",
            Self::Skip => "skip",
            Self::Error => "error",
        }
    }
}

impl std::fmt::Display for RuleOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse a rule outcome from a database string.
///
/// Logs a warning and defaults to `Error` for unrecognized values,
/// avoiding silent inflation of pass counts.
pub(crate) fn parse_rule_outcome(s: &str) -> RuleOutcome {
    match s {
        "pass" => RuleOutcome::Pass,
        "fail" => RuleOutcome::Fail,
        "skip" => RuleOutcome::Skip,
        "error" => RuleOutcome::Error,
        other => {
            tracing::warn!(value = other, "Unknown rule outcome in DB, defaulting to Error");
            RuleOutcome::Error
        }
    }
}

/// A single rule trigger event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleTriggerDetail {
    /// Unique identifier.
    pub id: i64,
    /// Name of the rule.
    pub rule_name: String,
    /// Rule category.
    pub category: Option<String>,
    /// Who or what triggered the rule.
    pub triggered_by: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Outcome of the rule trigger.
    pub outcome: RuleOutcome,
    /// Additional details.
    pub details: Option<String>,
    /// Timestamp of the event.
    pub created_at: String,
}

/// Effectiveness stats for a single rule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleEffectiveness {
    /// Name of the rule.
    pub rule_name: String,
    /// Total number of triggers.
    pub total_triggers: u64,
    /// Number of pass outcomes.
    pub pass_count: u64,
    /// Number of fail outcomes.
    pub fail_count: u64,
    /// Number of skip outcomes.
    pub skip_count: u64,
    /// Number of error outcomes.
    pub error_count: u64,
    /// Average duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Failure rate as percentage (0.0 - 100.0).
    pub failure_rate: f64,
}

/// Overall rule analytics summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleAnalyticsSummary {
    /// Total number of rule triggers.
    pub total_triggers: u64,
    /// Total number of pass outcomes.
    pub total_passes: u64,
    /// Total number of fail outcomes.
    pub total_failures: u64,
    /// Total number of skip outcomes.
    pub total_skips: u64,
    /// Total number of error outcomes.
    pub total_errors: u64,
    /// Number of unique rules.
    pub unique_rules: u64,
    /// Average duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Overall failure rate as percentage.
    pub overall_failure_rate: f64,
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
    /// Rule trigger event.
    RuleTrigger {
        /// Unique identifier.
        id: i64,
        /// Name of the rule.
        rule_name: String,
        /// Rule category.
        category: Option<String>,
        /// Outcome of the rule trigger.
        outcome: RuleOutcome,
        /// Duration in milliseconds.
        duration_ms: Option<u64>,
        /// Timestamp of the event.
        created_at: String,
    },
}

/// A single validation run detail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationDetail {
    /// Unique identifier.
    pub id: i64,
    /// Name of the skill validated.
    pub skill_name: String,
    /// Checks that passed.
    pub checks_passed: Vec<String>,
    /// Checks that failed.
    pub checks_failed: Vec<String>,
    /// Timestamp of the validation run.
    pub created_at: String,
}

impl ValidationDetail {
    /// Returns true if all checks passed (no failures).
    pub fn is_valid(&self) -> bool {
        self.checks_failed.is_empty()
    }

    /// Total number of checks executed.
    pub fn total_checks(&self) -> usize {
        self.checks_passed.len() + self.checks_failed.len()
    }
}

/// Summary of validation status across all skills.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidationSummary {
    /// Total number of unique skills with validation runs.
    pub total_skills: u64,
    /// Number of skills whose latest run passed all checks.
    pub valid: u64,
    /// Number of skills whose latest run had at least one warning (passed > 0 and failed > 0).
    pub warning: u64,
    /// Number of skills whose latest run had only failures (passed == 0 and failed > 0).
    pub error: u64,
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

/// A skill ranked by invocation count.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TopSkill {
    /// Skill name.
    pub skill_name: String,
    /// Total number of invocations.
    pub total_invocations: u64,
    /// Number of successful invocations.
    pub successful_invocations: u64,
    /// Number of failed invocations.
    pub failed_invocations: u64,
    /// Average duration in milliseconds.
    pub avg_duration_ms: f64,
}

/// Overall analytics summary across all skills.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnalyticsSummary {
    /// Total invocations across all skills.
    pub total_invocations: u64,
    /// Total successful invocations.
    pub successful_invocations: u64,
    /// Total failed invocations.
    pub failed_invocations: u64,
    /// Overall average duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Overall success rate as a percentage (0.0 - 100.0).
    pub success_rate: f64,
    /// Total tokens consumed across all invocations.
    pub total_tokens: u64,
    /// Number of unique skills that have been invoked.
    pub unique_skills: u64,
}

/// Detail record for a single sync event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncDetail {
    /// Unique identifier.
    pub id: i64,
    /// Operation type (push or pull).
    pub operation: SyncOperation,
    /// Number of files affected.
    pub files_count: usize,
    /// Status of the operation.
    pub status: SyncStatus,
    /// Timestamp of the event.
    pub created_at: String,
}

/// Aggregate summary of sync activity.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSummary {
    /// Total number of sync events recorded.
    pub total_syncs: u64,
    /// Number of successful syncs.
    pub successful_syncs: u64,
    /// Number of failed syncs.
    pub failed_syncs: u64,
    /// Success rate as a percentage (0.0 - 100.0).
    pub success_rate: f64,
    /// Total number of push operations.
    pub total_pushes: u64,
    /// Total number of pull operations.
    pub total_pulls: u64,
    /// Average files per sync operation.
    pub avg_files_per_sync: f64,
}
