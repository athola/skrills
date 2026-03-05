//! Metrics collector implementation.

use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use rusqlite::Connection;
use tokio::sync::broadcast::{self, Receiver, Sender};

use crate::error::{MetricsError, Result};
use crate::schema::init_schema;
use crate::types::{
    parse_sync_operation, parse_sync_status, AnalyticsSummary, MetricEvent, SkillStats, SyncDetail,
    SyncOperation, SyncStatus, SyncSummary, TopSkill, ValidationDetail, ValidationSummary,
};

/// Default channel capacity for metric event subscribers.
const CHANNEL_CAPACITY: usize = 100;

/// Storage mode for the metrics collector.
#[derive(Debug, Clone, Default)]
pub enum StorageMode {
    /// In-memory SQLite database (default, no persistence).
    #[default]
    InMemory,
    /// Persistent SQLite database at the specified path.
    Persistent(PathBuf),
}

/// Metrics collector that stores data in embedded SQLite.
///
/// By default uses in-memory storage. Use `persistent()` for file-based storage.
pub struct MetricsCollector {
    conn: Mutex<Connection>,
    sender: Sender<MetricEvent>,
    mode: StorageMode,
}

impl MetricsCollector {
    /// Create a new in-memory metrics collector (default).
    ///
    /// Data is lost when the collector is dropped.
    pub fn new() -> Result<Self> {
        Self::in_memory()
    }

    /// Create an in-memory metrics collector.
    ///
    /// Data is lost when the collector is dropped. Use `flush_to_disk()`
    /// to save data before dropping if needed.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        init_schema(&conn)?;

        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);

        Ok(Self {
            conn: Mutex::new(conn),
            sender,
            mode: StorageMode::InMemory,
        })
    }

    /// Create a persistent metrics collector at the specified path.
    ///
    /// Uses WAL mode for concurrent access.
    pub fn persistent(path: PathBuf) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(&path)?;

        // Enable WAL mode for concurrent access
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

        init_schema(&conn)?;

        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);

        Ok(Self {
            conn: Mutex::new(conn),
            sender,
            mode: StorageMode::Persistent(path),
        })
    }

    /// Create a persistent metrics collector at the default path.
    ///
    /// The database is stored at `~/.skrills/metrics.db`.
    pub fn persistent_default() -> Result<Self> {
        let path = Self::default_db_path()?;
        Self::persistent(path)
    }

    /// Get the default database path.
    fn default_db_path() -> Result<PathBuf> {
        let home = dirs::home_dir().ok_or(MetricsError::HomeNotFound)?;
        Ok(home.join(".skrills").join("metrics.db"))
    }

    /// Get the current storage mode.
    pub fn storage_mode(&self) -> &StorageMode {
        &self.mode
    }

    /// Flush in-memory database to disk.
    ///
    /// For in-memory collectors, this saves the database to the specified path.
    /// For persistent collectors, this is a no-op (data is already on disk).
    pub fn flush_to_disk(&self, path: &Path) -> Result<()> {
        // Reject paths with directory traversal components
        for component in path.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err(MetricsError::InvalidArgument(
                    "path must not contain '..' components".into(),
                ));
            }
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = self.conn.lock();
        conn.execute_batch(&format!(
            "VACUUM INTO '{}'",
            path.to_string_lossy().replace('\'', "''")
        ))?;
        Ok(())
    }

    /// Broadcast a metric event to all subscribers.
    ///
    /// Logs a warning if there are no active subscribers or if the channel is full.
    fn broadcast(&self, event: MetricEvent) {
        match self.sender.send(event) {
            Ok(_) => {}
            Err(_) => {
                // No active subscribers — this is normal when no dashboard is running.
                tracing::trace!("no active metric event subscribers");
            }
        }
    }

    /// Record a skill invocation.
    pub fn record_skill_invocation(
        &self,
        skill: &str,
        duration_ms: u64,
        success: bool,
        tokens: Option<u64>,
    ) -> Result<()> {
        self.record_skill_invocation_with_plugin(skill, None, duration_ms, success, tokens)
    }

    /// Record a skill invocation with plugin information.
    pub fn record_skill_invocation_with_plugin(
        &self,
        skill: &str,
        plugin: Option<&str>,
        duration_ms: u64,
        success: bool,
        tokens: Option<u64>,
    ) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO skill_invocations (skill_name, plugin, duration_ms, success, tokens_used) VALUES (?1, ?2, ?3, ?4, ?5)",
            (skill, plugin, duration_ms as i64, success as i32, tokens.map(|t| t as i64)),
        )?;

        let id = conn.last_insert_rowid();
        let created_at = conn.query_row(
            "SELECT created_at FROM skill_invocations WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )?;

        drop(conn);

        self.broadcast(MetricEvent::SkillInvocation {
            id,
            skill_name: skill.to_string(),
            plugin: plugin.map(String::from),
            duration_ms,
            success,
            tokens_used: tokens,
            created_at,
        });

        Ok(())
    }

    /// Record a validation run.
    pub fn record_validation(&self, skill: &str, passed: &[&str], failed: &[&str]) -> Result<()> {
        let passed_json = serde_json::to_string(passed)?;
        let failed_json = serde_json::to_string(failed)?;

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO validation_runs (skill_name, checks_passed, checks_failed) VALUES (?1, ?2, ?3)",
            (skill, &passed_json, &failed_json),
        )?;

        let id = conn.last_insert_rowid();
        let created_at = conn.query_row(
            "SELECT created_at FROM validation_runs WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )?;

        drop(conn);

        self.broadcast(MetricEvent::Validation {
            id,
            skill_name: skill.to_string(),
            checks_passed: passed.iter().map(|s| s.to_string()).collect(),
            checks_failed: failed.iter().map(|s| s.to_string()).collect(),
            created_at,
        });

        Ok(())
    }

    /// Record a sync event.
    pub fn record_sync_event(
        &self,
        operation: SyncOperation,
        files: usize,
        status: SyncStatus,
    ) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO sync_events (operation, files_count, status) VALUES (?1, ?2, ?3)",
            (operation.as_str(), files as i64, status.as_str()),
        )?;

        let id = conn.last_insert_rowid();
        let created_at = conn.query_row(
            "SELECT created_at FROM sync_events WHERE id = ?1",
            [id],
            |row| row.get::<_, String>(0),
        )?;

        drop(conn);

        self.broadcast(MetricEvent::Sync {
            id,
            operation,
            files_count: files,
            status,
            created_at,
        });

        Ok(())
    }

    /// Get recent metric events across all tables.
    ///
    /// Fetches up to `limit` rows from each event table independently, then merges
    /// and sorts by timestamp to return the global top-N. This approach is correct
    /// because each per-table query returns the most recent entries, guaranteeing
    /// the merged result contains the true top-N across all tables.
    pub fn get_recent_events(&self, limit: usize) -> Result<Vec<MetricEvent>> {
        let conn = self.conn.lock();
        let mut events = Vec::new();

        // Get recent skill invocations
        let mut stmt = conn.prepare(
            "SELECT id, skill_name, plugin, duration_ms, success, tokens_used, created_at
             FROM skill_invocations ORDER BY created_at DESC LIMIT ?1",
        )?;
        let invocations = stmt.query_map([limit as i64], |row| {
            Ok(MetricEvent::SkillInvocation {
                id: row.get(0)?,
                skill_name: row.get(1)?,
                plugin: row.get(2)?,
                duration_ms: row.get::<_, i64>(3)? as u64,
                success: row.get::<_, i32>(4)? != 0,
                tokens_used: row.get::<_, Option<i64>>(5)?.map(|t| t as u64),
                created_at: row.get(6)?,
            })
        })?;
        for inv in invocations {
            events.push(inv?);
        }

        // Get recent validations
        let mut stmt = conn.prepare(
            "SELECT id, skill_name, checks_passed, checks_failed, created_at
             FROM validation_runs ORDER BY created_at DESC LIMIT ?1",
        )?;
        let validations = stmt.query_map([limit as i64], |row| {
            let passed_json: String = row.get(2)?;
            let failed_json: String = row.get(3)?;
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                passed_json,
                failed_json,
                row.get::<_, String>(4)?,
            ))
        })?;
        for val in validations {
            let (id, skill_name, passed_json, failed_json, created_at) = val?;
            let checks_passed: Vec<String> = match serde_json::from_str(&passed_json) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        id,
                        skill_name = %skill_name,
                        error = %e,
                        "failed to deserialize checks_passed JSON, skipping row"
                    );
                    continue;
                }
            };
            let checks_failed: Vec<String> = match serde_json::from_str(&failed_json) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        id,
                        skill_name = %skill_name,
                        error = %e,
                        "failed to deserialize checks_failed JSON, skipping row"
                    );
                    continue;
                }
            };
            events.push(MetricEvent::Validation {
                id,
                skill_name,
                checks_passed,
                checks_failed,
                created_at,
            });
        }

        // Get recent sync events
        let mut stmt = conn.prepare(
            "SELECT id, operation, files_count, status, created_at
             FROM sync_events ORDER BY created_at DESC LIMIT ?1",
        )?;
        let syncs = stmt.query_map([limit as i64], |row| {
            let op_str: String = row.get(1)?;
            let status_str: String = row.get(3)?;
            Ok(MetricEvent::Sync {
                id: row.get(0)?,
                operation: parse_sync_operation(&op_str),
                files_count: row.get::<_, i64>(2)? as usize,
                status: parse_sync_status(&status_str),
                created_at: row.get(4)?,
            })
        })?;
        for sync in syncs {
            events.push(sync?);
        }

        // Sort by created_at descending and take limit
        events.sort_by(|a, b| {
            let a_time = match a {
                MetricEvent::SkillInvocation { created_at, .. } => created_at,
                MetricEvent::Validation { created_at, .. } => created_at,
                MetricEvent::Sync { created_at, .. } => created_at,
            };
            let b_time = match b {
                MetricEvent::SkillInvocation { created_at, .. } => created_at,
                MetricEvent::Validation { created_at, .. } => created_at,
                MetricEvent::Sync { created_at, .. } => created_at,
            };
            b_time.cmp(a_time)
        });

        events.truncate(limit);
        Ok(events)
    }

    /// Get statistics for a specific skill.
    pub fn get_skill_stats(&self, skill: &str) -> Result<SkillStats> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT
                COALESCE(SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END), 0) as successful,
                COALESCE(SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END), 0) as failed,
                AVG(duration_ms) as avg_duration,
                COALESCE(SUM(tokens_used), 0) as total_tokens
             FROM skill_invocations WHERE skill_name = ?1",
        )?;

        let stats = stmt.query_row([skill], |row| {
            Ok(SkillStats {
                successful_invocations: row.get::<_, Option<i64>>(0)?.unwrap_or(0) as u64,
                failed_invocations: row.get::<_, Option<i64>>(1)?.unwrap_or(0) as u64,
                avg_duration_ms: row.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                total_tokens: row.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
            })
        })?;

        Ok(stats)
    }

    /// Get validation history for a specific skill.
    ///
    /// Returns up to `limit` most recent validation runs, ordered by timestamp descending.
    pub fn get_validation_history(&self, skill: &str, limit: usize) -> Result<Vec<ValidationDetail>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, skill_name, checks_passed, checks_failed, created_at
             FROM validation_runs WHERE skill_name = ?1
             ORDER BY created_at DESC, id DESC LIMIT ?2",
        )?;

        let rows = stmt.query_map(rusqlite::params![skill, limit as i64], |row| {
            let passed_json: String = row.get(2)?;
            let failed_json: String = row.get(3)?;
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                passed_json,
                failed_json,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut details = Vec::new();
        for row in rows {
            let (id, skill_name, passed_json, failed_json, created_at) = row?;
            let checks_passed: Vec<String> = serde_json::from_str(&passed_json)?;
            let checks_failed: Vec<String> = serde_json::from_str(&failed_json)?;
            details.push(ValidationDetail {
                id,
                skill_name,
                checks_passed,
                checks_failed,
                created_at,
            });
        }

        Ok(details)
    }

    /// Get a summary of validation status across all skills.
    ///
    /// For each skill, examines the most recent validation run to classify it as
    /// valid (all passed), warning (some passed, some failed), or error (none passed).
    pub fn get_validation_summary(&self) -> Result<ValidationSummary> {
        let conn = self.conn.lock();

        // Use a window function to get only the latest run per skill
        let mut stmt = conn.prepare(
            "SELECT skill_name, checks_passed, checks_failed
             FROM (
                 SELECT skill_name, checks_passed, checks_failed,
                        ROW_NUMBER() OVER (PARTITION BY skill_name ORDER BY created_at DESC, id DESC) as rn
                 FROM validation_runs
             ) WHERE rn = 1",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

        let mut summary = ValidationSummary::default();

        for row in rows {
            let (_skill_name, passed_json, failed_json) = row?;
            let passed: Vec<String> = serde_json::from_str(&passed_json).unwrap_or_default();
            let failed: Vec<String> = serde_json::from_str(&failed_json).unwrap_or_default();

            summary.total_skills += 1;
            if failed.is_empty() {
                summary.valid += 1;
            } else if passed.is_empty() {
                summary.error += 1;
            } else {
                summary.warning += 1;
            }
        }

        Ok(summary)
    }

    /// Export a validation report as JSON.
    ///
    /// Returns a JSON object containing the validation summary and per-skill
    /// latest validation details.
    pub fn export_validation_report(&self) -> Result<serde_json::Value> {
        let summary = self.get_validation_summary()?;

        let conn = self.conn.lock();

        // Get the latest validation run for each skill
        let mut stmt = conn.prepare(
            "SELECT id, skill_name, checks_passed, checks_failed, created_at
             FROM (
                 SELECT id, skill_name, checks_passed, checks_failed, created_at,
                        ROW_NUMBER() OVER (PARTITION BY skill_name ORDER BY created_at DESC, id DESC) as rn
                 FROM validation_runs
             ) WHERE rn = 1
             ORDER BY skill_name",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;

        let mut skills = Vec::new();
        for row in rows {
            let (id, skill_name, passed_json, failed_json, created_at) = row?;
            let checks_passed: Vec<String> = serde_json::from_str(&passed_json).unwrap_or_default();
            let checks_failed: Vec<String> = serde_json::from_str(&failed_json).unwrap_or_default();

            let status = if checks_failed.is_empty() {
                "valid"
            } else if checks_passed.is_empty() {
                "error"
            } else {
                "warning"
            };

            skills.push(serde_json::json!({
                "id": id,
                "skill_name": skill_name,
                "status": status,
                "checks_passed": checks_passed,
                "checks_failed": checks_failed,
                "created_at": created_at,
            }));
        }

        Ok(serde_json::json!({
            "summary": summary,
            "skills": skills,
        }))
    }

    /// Get the top skills by invocation count.
    ///
    /// Returns up to `limit` skills ordered by total invocations descending.
    pub fn get_top_skills(&self, limit: usize) -> Result<Vec<TopSkill>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT
                skill_name,
                COUNT(*) as total,
                COALESCE(SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END), 0) as successful,
                COALESCE(SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END), 0) as failed,
                AVG(duration_ms) as avg_duration
             FROM skill_invocations
             GROUP BY skill_name
             ORDER BY total DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map([limit as i64], |row| {
            Ok(TopSkill {
                skill_name: row.get(0)?,
                total_invocations: row.get::<_, i64>(1)? as u64,
                successful_invocations: row.get::<_, i64>(2)? as u64,
                failed_invocations: row.get::<_, i64>(3)? as u64,
                avg_duration_ms: row.get::<_, f64>(4)?,
            })
        })?;

        let mut skills = Vec::new();
        for row in rows {
            skills.push(row?);
        }

        Ok(skills)
    }

    /// Get an overall analytics summary across all skills.
    ///
    /// Aggregates total invocations, success rate, average duration, total tokens,
    /// and unique skill count from the skill_invocations table.
    pub fn get_analytics_summary(&self) -> Result<AnalyticsSummary> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT
                COUNT(*) as total,
                COALESCE(SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END), 0) as successful,
                COALESCE(SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END), 0) as failed,
                COALESCE(AVG(duration_ms), 0.0) as avg_duration,
                COALESCE(SUM(tokens_used), 0) as total_tokens,
                COUNT(DISTINCT skill_name) as unique_skills
             FROM skill_invocations",
        )?;

        let summary = stmt.query_row([], |row| {
            let total: i64 = row.get(0)?;
            let successful: i64 = row.get(1)?;
            let success_rate = if total > 0 {
                (successful as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            Ok(AnalyticsSummary {
                total_invocations: total as u64,
                successful_invocations: successful as u64,
                failed_invocations: row.get::<_, i64>(2)? as u64,
                avg_duration_ms: row.get(3)?,
                success_rate,
                total_tokens: row.get::<_, i64>(4)? as u64,
                unique_skills: row.get::<_, i64>(5)? as u64,
            })
        })?;

        Ok(summary)
    }

    /// Get recent sync event history.
    ///
    /// Returns up to `limit` most recent sync events, ordered by timestamp descending.
    pub fn get_sync_history(&self, limit: usize) -> Result<Vec<SyncDetail>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT id, operation, files_count, status, created_at
             FROM sync_events ORDER BY created_at DESC LIMIT ?1",
        )?;

        let rows = stmt.query_map([limit as i64], |row| {
            let op_str: String = row.get(1)?;
            let status_str: String = row.get(3)?;
            Ok(SyncDetail {
                id: row.get(0)?,
                operation: parse_sync_operation(&op_str),
                files_count: row.get::<_, i64>(2)? as usize,
                status: parse_sync_status(&status_str),
                created_at: row.get(4)?,
            })
        })?;

        let mut details = Vec::new();
        for row in rows {
            details.push(row?);
        }

        Ok(details)
    }

    /// Get an aggregate summary of all sync activity.
    ///
    /// Returns totals, success rate, push/pull breakdown, and average files per sync.
    pub fn get_sync_summary(&self) -> Result<SyncSummary> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT
                COUNT(*) as total,
                COALESCE(SUM(CASE WHEN status = 'success' OR status = 'complete' THEN 1 ELSE 0 END), 0) as successful,
                COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0) as failed,
                COALESCE(SUM(CASE WHEN operation = 'push' THEN 1 ELSE 0 END), 0) as pushes,
                COALESCE(SUM(CASE WHEN operation = 'pull' THEN 1 ELSE 0 END), 0) as pulls,
                COALESCE(AVG(files_count), 0.0) as avg_files
             FROM sync_events",
        )?;

        let summary = stmt.query_row([], |row| {
            let total: i64 = row.get(0)?;
            let successful: i64 = row.get(1)?;
            let failed: i64 = row.get(2)?;
            let pushes: i64 = row.get(3)?;
            let pulls: i64 = row.get(4)?;
            let avg_files: f64 = row.get(5)?;

            let success_rate = if total > 0 {
                (successful as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            Ok(SyncSummary {
                total_syncs: total as u64,
                successful_syncs: successful as u64,
                failed_syncs: failed as u64,
                success_rate,
                total_pushes: pushes as u64,
                total_pulls: pulls as u64,
                avg_files_per_sync: avg_files,
            })
        })?;

        Ok(summary)
    }

    /// Subscribe to metric events.
    pub fn subscribe(&self) -> Receiver<MetricEvent> {
        self.sender.subscribe()
    }

    /// Clean up data older than the specified number of days.
    ///
    /// Returns the total number of rows deleted.
    pub fn cleanup_old_data(&self, days: u32) -> Result<usize> {
        if days == 0 || days > 3650 {
            return Err(MetricsError::InvalidArgument(format!(
                "days must be between 1 and 3650, got {days}"
            )));
        }

        let conn = self.conn.lock();
        let cutoff = format!("-{} days", days);

        let mut total_deleted = 0usize;

        total_deleted += conn.execute(
            "DELETE FROM skill_invocations WHERE created_at < datetime('now', ?1)",
            [&cutoff],
        )?;

        total_deleted += conn.execute(
            "DELETE FROM validation_runs WHERE created_at < datetime('now', ?1)",
            [&cutoff],
        )?;

        total_deleted += conn.execute(
            "DELETE FROM sync_events WHERE created_at < datetime('now', ?1)",
            [&cutoff],
        )?;

        Ok(total_deleted)
    }

    /// Apply default retention policy (30 days).
    pub fn apply_retention_policy(&self) -> Result<usize> {
        self.cleanup_old_data(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_in_memory() {
        let collector = MetricsCollector::new().unwrap();
        assert!(matches!(collector.storage_mode(), StorageMode::InMemory));
    }

    #[test]
    fn test_record_skill_invocation() {
        let collector = MetricsCollector::in_memory().unwrap();
        collector
            .record_skill_invocation("test-skill", 150, true, Some(1024))
            .unwrap();

        let stats = collector.get_skill_stats("test-skill").unwrap();
        assert_eq!(stats.total_invocations(), 1);
        assert_eq!(stats.successful_invocations, 1);
        assert_eq!(stats.failed_invocations, 0);
        assert_eq!(stats.total_tokens, 1024);
    }

    #[test]
    fn test_record_skill_invocation_with_plugin() {
        let collector = MetricsCollector::in_memory().unwrap();
        collector
            .record_skill_invocation_with_plugin("test-skill", Some("my-plugin"), 200, false, None)
            .unwrap();

        let stats = collector.get_skill_stats("test-skill").unwrap();
        assert_eq!(stats.total_invocations(), 1);
        assert_eq!(stats.failed_invocations, 1);
    }

    #[test]
    fn test_record_validation() {
        let collector = MetricsCollector::in_memory().unwrap();
        collector
            .record_validation("test-skill", &["check1", "check2"], &["check3"])
            .unwrap();

        let events = collector.get_recent_events(10).unwrap();
        assert_eq!(events.len(), 1);

        if let MetricEvent::Validation {
            checks_passed,
            checks_failed,
            ..
        } = &events[0]
        {
            assert_eq!(checks_passed.len(), 2);
            assert_eq!(checks_failed.len(), 1);
        } else {
            panic!("Expected Validation event");
        }
    }

    #[test]
    fn test_record_sync_event() {
        let collector = MetricsCollector::in_memory().unwrap();
        collector
            .record_sync_event(SyncOperation::Push, 5, SyncStatus::Success)
            .unwrap();

        let events = collector.get_recent_events(10).unwrap();
        assert_eq!(events.len(), 1);

        if let MetricEvent::Sync {
            operation,
            files_count,
            status,
            ..
        } = &events[0]
        {
            assert_eq!(*operation, SyncOperation::Push);
            assert_eq!(*files_count, 5);
            assert_eq!(*status, SyncStatus::Success);
        } else {
            panic!("Expected Sync event");
        }
    }

    #[test]
    fn test_get_recent_events_mixed() {
        let collector = MetricsCollector::in_memory().unwrap();

        collector
            .record_skill_invocation("skill1", 100, true, None)
            .unwrap();
        collector.record_validation("skill2", &["a"], &[]).unwrap();
        collector
            .record_sync_event(SyncOperation::Pull, 3, SyncStatus::Success)
            .unwrap();

        let events = collector.get_recent_events(10).unwrap();
        assert_eq!(events.len(), 3);
    }

    #[test]
    fn test_get_skill_stats_multiple_invocations() {
        let collector = MetricsCollector::in_memory().unwrap();

        collector
            .record_skill_invocation("multi-skill", 100, true, Some(500))
            .unwrap();
        collector
            .record_skill_invocation("multi-skill", 200, true, Some(600))
            .unwrap();
        collector
            .record_skill_invocation("multi-skill", 150, false, Some(400))
            .unwrap();

        let stats = collector.get_skill_stats("multi-skill").unwrap();
        assert_eq!(stats.total_invocations(), 3);
        assert_eq!(stats.successful_invocations, 2);
        assert_eq!(stats.failed_invocations, 1);
        assert_eq!(stats.total_tokens, 1500);
        assert!((stats.avg_duration_ms - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_get_skill_stats_nonexistent() {
        let collector = MetricsCollector::in_memory().unwrap();
        let stats = collector.get_skill_stats("nonexistent").unwrap();
        assert_eq!(stats.total_invocations(), 0);
        assert_eq!(stats.successful_invocations, 0);
        assert_eq!(stats.failed_invocations, 0);
    }

    #[test]
    fn test_cleanup_old_data() {
        let collector = MetricsCollector::in_memory().unwrap();

        // Insert data with explicit old timestamp
        {
            let conn = collector.conn.lock();
            conn.execute(
                "INSERT INTO skill_invocations (skill_name, duration_ms, success, created_at)
                 VALUES ('old-skill', 100, 1, datetime('now', '-31 days'))",
                [],
            )
            .unwrap();
        }

        // Cleanup with 30 days should delete old data
        let deleted = collector.cleanup_old_data(30).unwrap();
        assert_eq!(deleted, 1);

        let events = collector.get_recent_events(10).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn test_cleanup_preserves_recent() {
        let collector = MetricsCollector::in_memory().unwrap();

        collector
            .record_skill_invocation("recent-skill", 100, true, None)
            .unwrap();

        // Cleanup with 30 days should not delete recent data
        let deleted = collector.cleanup_old_data(30).unwrap();
        assert_eq!(deleted, 0);

        let stats = collector.get_skill_stats("recent-skill").unwrap();
        assert_eq!(stats.total_invocations(), 1);
    }

    #[tokio::test]
    async fn test_subscribe() {
        let collector = MetricsCollector::in_memory().unwrap();
        let mut rx = collector.subscribe();

        collector
            .record_skill_invocation("sub-skill", 100, true, None)
            .unwrap();

        let event = rx.try_recv().unwrap();
        if let MetricEvent::SkillInvocation { skill_name, .. } = event {
            assert_eq!(skill_name, "sub-skill");
        } else {
            panic!("Expected SkillInvocation event");
        }
    }

    #[test]
    fn test_recent_events_limit() {
        let collector = MetricsCollector::in_memory().unwrap();

        for i in 0..10 {
            collector
                .record_skill_invocation(&format!("skill-{}", i), 100, true, None)
                .unwrap();
        }

        let events = collector.get_recent_events(5).unwrap();
        assert_eq!(events.len(), 5);
    }

    #[test]
    fn test_persistent_storage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test_metrics.db");

        // Create and populate
        {
            let collector = MetricsCollector::persistent(db_path.clone()).unwrap();
            assert!(matches!(
                collector.storage_mode(),
                StorageMode::Persistent(_)
            ));
            collector
                .record_skill_invocation("persistent-skill", 100, true, None)
                .unwrap();
        }

        // Reopen and verify
        {
            let collector = MetricsCollector::persistent(db_path).unwrap();
            let stats = collector.get_skill_stats("persistent-skill").unwrap();
            assert_eq!(stats.total_invocations(), 1);
        }
    }

    #[test]
    fn test_flush_to_disk() {
        let collector = MetricsCollector::in_memory().unwrap();
        collector
            .record_skill_invocation("flush-skill", 100, true, None)
            .unwrap();

        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("flushed.db");

        collector.flush_to_disk(&db_path).unwrap();

        // Verify the flushed database
        let persistent = MetricsCollector::persistent(db_path).unwrap();
        let stats = persistent.get_skill_stats("flush-skill").unwrap();
        assert_eq!(stats.total_invocations(), 1);
    }

    #[test]
    fn test_flush_to_disk_rejects_path_traversal() {
        let collector = MetricsCollector::in_memory().unwrap();
        let malicious = Path::new("/tmp/../etc/passwd");
        let result = collector.flush_to_disk(malicious);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, MetricsError::InvalidArgument(_)),
            "expected InvalidArgument, got: {err:?}"
        );
    }

    #[test]
    fn test_flush_to_disk_allows_normal_paths() {
        let collector = MetricsCollector::in_memory().unwrap();
        let temp_dir = tempfile::tempdir().unwrap();
        let normal_path = temp_dir.path().join("subdir").join("metrics.db");
        assert!(collector.flush_to_disk(&normal_path).is_ok());
    }

    #[test]
    fn test_cleanup_old_data_rejects_zero_days() {
        let collector = MetricsCollector::in_memory().unwrap();
        let result = collector.cleanup_old_data(0);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, MetricsError::InvalidArgument(_)),
            "expected InvalidArgument, got: {err:?}"
        );
    }

    #[test]
    fn test_cleanup_old_data_rejects_excessive_days() {
        let collector = MetricsCollector::in_memory().unwrap();
        let result = collector.cleanup_old_data(9999);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, MetricsError::InvalidArgument(_)),
            "expected InvalidArgument, got: {err:?}"
        );
    }

    #[test]
    fn test_cleanup_old_data_accepts_boundary_values() {
        let collector = MetricsCollector::in_memory().unwrap();
        // 1 day (lower bound) should succeed
        assert!(collector.cleanup_old_data(1).is_ok());
        // 3650 days (upper bound) should succeed
        assert!(collector.cleanup_old_data(3650).is_ok());
    }

    #[test]
    fn test_get_top_skills() {
        let collector = MetricsCollector::in_memory().unwrap();

        // Record invocations for multiple skills
        for _ in 0..5 {
            collector
                .record_skill_invocation("popular-skill", 100, true, Some(100))
                .unwrap();
        }
        for _ in 0..3 {
            collector
                .record_skill_invocation("medium-skill", 200, true, Some(200))
                .unwrap();
        }
        collector
            .record_skill_invocation("rare-skill", 300, false, Some(300))
            .unwrap();

        let top = collector.get_top_skills(2).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].skill_name, "popular-skill");
        assert_eq!(top[0].total_invocations, 5);
        assert_eq!(top[0].successful_invocations, 5);
        assert_eq!(top[0].failed_invocations, 0);
        assert_eq!(top[1].skill_name, "medium-skill");
        assert_eq!(top[1].total_invocations, 3);

        // Request all
        let all = collector.get_top_skills(10).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_get_top_skills_empty() {
        let collector = MetricsCollector::in_memory().unwrap();
        let top = collector.get_top_skills(5).unwrap();
        assert!(top.is_empty());
    }

    #[test]
    fn test_get_analytics_summary() {
        let collector = MetricsCollector::in_memory().unwrap();

        collector
            .record_skill_invocation("skill-a", 100, true, Some(500))
            .unwrap();
        collector
            .record_skill_invocation("skill-a", 200, true, Some(600))
            .unwrap();
        collector
            .record_skill_invocation("skill-b", 150, false, Some(400))
            .unwrap();

        let summary = collector.get_analytics_summary().unwrap();
        assert_eq!(summary.total_invocations, 3);
        assert_eq!(summary.successful_invocations, 2);
        assert_eq!(summary.failed_invocations, 1);
        assert_eq!(summary.total_tokens, 1500);
        assert_eq!(summary.unique_skills, 2);
        assert!((summary.success_rate - 66.666).abs() < 1.0);
        assert!((summary.avg_duration_ms - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_get_analytics_summary_empty() {
        let collector = MetricsCollector::in_memory().unwrap();
        let summary = collector.get_analytics_summary().unwrap();
        assert_eq!(summary.total_invocations, 0);
        assert_eq!(summary.successful_invocations, 0);
        assert_eq!(summary.failed_invocations, 0);
        assert_eq!(summary.total_tokens, 0);
        assert_eq!(summary.unique_skills, 0);
        assert!((summary.success_rate - 0.0).abs() < 0.01);
        assert!((summary.avg_duration_ms - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_get_validation_history() {
        let collector = MetricsCollector::in_memory().unwrap();

        collector
            .record_validation("skill-a", &["check1", "check2"], &["check3"])
            .unwrap();
        collector
            .record_validation("skill-a", &["check1", "check2", "check3"], &[])
            .unwrap();
        collector
            .record_validation("skill-b", &["check1"], &["check2"])
            .unwrap();

        // Get history for skill-a (should be 2, most recent first)
        let history = collector.get_validation_history("skill-a", 10).unwrap();
        assert_eq!(history.len(), 2);
        // Most recent first: the one with all passing
        assert!(history[0].checks_failed.is_empty());
        assert_eq!(history[0].checks_passed.len(), 3);
        // Older one had a failure
        assert_eq!(history[1].checks_failed.len(), 1);

        // Limit works
        let limited = collector.get_validation_history("skill-a", 1).unwrap();
        assert_eq!(limited.len(), 1);

        // Non-existent skill returns empty
        let empty = collector
            .get_validation_history("no-such-skill", 10)
            .unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_get_validation_summary() {
        let collector = MetricsCollector::in_memory().unwrap();

        // skill-a: all pass (valid)
        collector
            .record_validation("skill-a", &["check1", "check2"], &[])
            .unwrap();
        // skill-b: some pass some fail (warning)
        collector
            .record_validation("skill-b", &["check1"], &["check2"])
            .unwrap();
        // skill-c: all fail (error)
        collector
            .record_validation("skill-c", &[], &["check1", "check2"])
            .unwrap();

        let summary = collector.get_validation_summary().unwrap();
        assert_eq!(summary.total_skills, 3);
        assert_eq!(summary.valid, 1);
        assert_eq!(summary.warning, 1);
        assert_eq!(summary.error, 1);
    }

    #[test]
    fn test_get_validation_summary_uses_latest_run() {
        let collector = MetricsCollector::in_memory().unwrap();

        // skill-a first fails, then passes — summary should show valid
        collector
            .record_validation("skill-a", &[], &["check1"])
            .unwrap();
        collector
            .record_validation("skill-a", &["check1"], &[])
            .unwrap();

        let summary = collector.get_validation_summary().unwrap();
        assert_eq!(summary.total_skills, 1);
        assert_eq!(summary.valid, 1);
        assert_eq!(summary.error, 0);
    }

    #[test]
    fn test_get_validation_summary_empty() {
        let collector = MetricsCollector::in_memory().unwrap();

        let summary = collector.get_validation_summary().unwrap();
        assert_eq!(summary.total_skills, 0);
        assert_eq!(summary.valid, 0);
        assert_eq!(summary.warning, 0);
        assert_eq!(summary.error, 0);
    }

    #[test]
    fn test_export_validation_report() {
        let collector = MetricsCollector::in_memory().unwrap();

        collector
            .record_validation("skill-a", &["check1"], &[])
            .unwrap();
        collector
            .record_validation("skill-b", &["check1"], &["check2"])
            .unwrap();

        let report = collector.export_validation_report().unwrap();

        // Check summary section
        let summary = &report["summary"];
        assert_eq!(summary["total_skills"], 2);
        assert_eq!(summary["valid"], 1);
        assert_eq!(summary["warning"], 1);
        assert_eq!(summary["error"], 0);

        // Check skills section
        let skills = report["skills"].as_array().unwrap();
        assert_eq!(skills.len(), 2);

        // Skills are ordered alphabetically by name
        assert_eq!(skills[0]["skill_name"], "skill-a");
        assert_eq!(skills[0]["status"], "valid");
        assert_eq!(skills[1]["skill_name"], "skill-b");
        assert_eq!(skills[1]["status"], "warning");
    }

    #[test]
    fn test_export_validation_report_empty() {
        let collector = MetricsCollector::in_memory().unwrap();

        let report = collector.export_validation_report().unwrap();

        let summary = &report["summary"];
        assert_eq!(summary["total_skills"], 0);
        let skills = report["skills"].as_array().unwrap();
        assert!(skills.is_empty());
    }

    #[test]
    fn test_validation_detail_helpers() {
        let detail = ValidationDetail {
            id: 1,
            skill_name: "test".to_string(),
            checks_passed: vec!["a".to_string(), "b".to_string()],
            checks_failed: vec!["c".to_string()],
            created_at: "2025-01-01T00:00:00".to_string(),
        };
        assert!(!detail.is_valid());
        assert_eq!(detail.total_checks(), 3);

        let valid_detail = ValidationDetail {
            id: 2,
            skill_name: "test".to_string(),
            checks_passed: vec!["a".to_string()],
            checks_failed: vec![],
            created_at: "2025-01-01T00:00:00".to_string(),
        };
        assert!(valid_detail.is_valid());
        assert_eq!(valid_detail.total_checks(), 1);
    }

    #[test]
    fn test_get_sync_history() {
        let collector = MetricsCollector::in_memory().unwrap();

        collector
            .record_sync_event(SyncOperation::Push, 5, SyncStatus::Success)
            .unwrap();
        collector
            .record_sync_event(SyncOperation::Pull, 3, SyncStatus::Failed)
            .unwrap();
        collector
            .record_sync_event(SyncOperation::Push, 10, SyncStatus::Complete)
            .unwrap();

        // Get all history
        let history = collector.get_sync_history(10).unwrap();
        assert_eq!(history.len(), 3);
        // Most recent first
        assert_eq!(history[0].files_count, 10);
        assert_eq!(history[0].operation, SyncOperation::Push);
        assert_eq!(history[0].status, SyncStatus::Complete);

        // Limit works
        let limited = collector.get_sync_history(2).unwrap();
        assert_eq!(limited.len(), 2);
    }

    #[test]
    fn test_get_sync_history_empty() {
        let collector = MetricsCollector::in_memory().unwrap();
        let history = collector.get_sync_history(10).unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn test_get_sync_summary() {
        let collector = MetricsCollector::in_memory().unwrap();

        collector
            .record_sync_event(SyncOperation::Push, 5, SyncStatus::Success)
            .unwrap();
        collector
            .record_sync_event(SyncOperation::Push, 3, SyncStatus::Failed)
            .unwrap();
        collector
            .record_sync_event(SyncOperation::Pull, 10, SyncStatus::Complete)
            .unwrap();
        collector
            .record_sync_event(SyncOperation::Pull, 2, SyncStatus::Success)
            .unwrap();

        let summary = collector.get_sync_summary().unwrap();
        assert_eq!(summary.total_syncs, 4);
        assert_eq!(summary.successful_syncs, 3); // success + complete
        assert_eq!(summary.failed_syncs, 1);
        assert_eq!(summary.total_pushes, 2);
        assert_eq!(summary.total_pulls, 2);
        assert!((summary.success_rate - 75.0).abs() < 0.01);
        assert!((summary.avg_files_per_sync - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_get_sync_summary_empty() {
        let collector = MetricsCollector::in_memory().unwrap();
        let summary = collector.get_sync_summary().unwrap();
        assert_eq!(summary.total_syncs, 0);
        assert_eq!(summary.successful_syncs, 0);
        assert_eq!(summary.failed_syncs, 0);
        assert!((summary.success_rate - 0.0).abs() < 0.01);
        assert!((summary.avg_files_per_sync - 0.0).abs() < 0.01);
    }
}
