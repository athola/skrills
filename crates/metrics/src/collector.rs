//! Metrics collector implementation.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;
use tokio::sync::broadcast::{self, Receiver, Sender};

use crate::error::{MetricsError, Result};
use crate::schema::init_schema;
use crate::types::{parse_sync_operation, parse_sync_status, MetricEvent, SkillStats, SyncOperation, SyncStatus};

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

        let conn = self.conn.lock().map_err(|_| MetricsError::MutexPoisoned)?;
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
        let conn = self.conn.lock().map_err(|_| MetricsError::MutexPoisoned)?;
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

        let conn = self.conn.lock().map_err(|_| MetricsError::MutexPoisoned)?;
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
        let conn = self.conn.lock().map_err(|_| MetricsError::MutexPoisoned)?;
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
        let conn = self.conn.lock().map_err(|_| MetricsError::MutexPoisoned)?;
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
        let conn = self.conn.lock().map_err(|_| MetricsError::MutexPoisoned)?;
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

        let conn = self.conn.lock().map_err(|_| MetricsError::MutexPoisoned)?;
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
            let conn = collector.conn.lock().unwrap();
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
}
