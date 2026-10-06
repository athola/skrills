//! Database schema initialization and migrations.

use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::Result;

/// SQL statements to create the initial metrics schema (version 1).
const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS skill_invocations (
    id INTEGER PRIMARY KEY,
    skill_name TEXT NOT NULL,
    plugin TEXT,
    duration_ms INTEGER,
    success INTEGER,
    tokens_used INTEGER,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS validation_runs (
    id INTEGER PRIMARY KEY,
    skill_name TEXT NOT NULL,
    checks_passed TEXT,
    checks_failed TEXT,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS sync_events (
    id INTEGER PRIMARY KEY,
    operation TEXT NOT NULL,
    files_count INTEGER,
    status TEXT,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_invocations_skill ON skill_invocations(skill_name);
CREATE INDEX IF NOT EXISTS idx_invocations_time ON skill_invocations(created_at);
CREATE INDEX IF NOT EXISTS idx_validation_skill ON validation_runs(skill_name);
CREATE INDEX IF NOT EXISTS idx_validation_time ON validation_runs(created_at);
CREATE INDEX IF NOT EXISTS idx_sync_time ON sync_events(created_at);
"#;

/// SQL statements for the V2 migration (rule triggers table).
const SCHEMA_V2: &str = r#"
CREATE TABLE IF NOT EXISTS rule_triggers (
    id INTEGER PRIMARY KEY,
    rule_name TEXT NOT NULL,
    category TEXT,
    triggered_by TEXT,
    duration_ms INTEGER,
    outcome TEXT NOT NULL DEFAULT 'pass',
    details TEXT,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_rule_triggers_name ON rule_triggers(rule_name);
CREATE INDEX IF NOT EXISTS idx_rule_triggers_time ON rule_triggers(created_at);
CREATE INDEX IF NOT EXISTS idx_rule_triggers_category ON rule_triggers(category);
"#;

/// Migrations in order: entry `i` brings the schema to version `i + 1`.
/// Append new migrations here; the current version follows from the length.
const MIGRATIONS: &[&str] = &[SCHEMA_V1, SCHEMA_V2];

/// Current schema version: the number of migrations.
const SCHEMA_VERSION: i32 = MIGRATIONS.len() as i32;

/// Initialize the database schema with versioned migrations.
///
/// Creates a `schema_version` table to track the current version, then
/// applies any pending migrations in order. The version read and the
/// migrations run in one `BEGIN IMMEDIATE` transaction, so two processes
/// opening a fresh database cannot both apply the same migration.
pub fn init_schema(conn: &Connection) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        )",
    )?;

    let current: i32 = tx.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_version",
        [],
        |row| row.get(0),
    )?;

    for (version, migration) in (1..=SCHEMA_VERSION).zip(MIGRATIONS) {
        if current < version {
            tx.execute_batch(migration)?;
            tx.execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
                [version],
            )?;
        }
    }

    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_creation() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();

        assert!(tables.contains(&"skill_invocations".to_string()));
        assert!(tables.contains(&"validation_runs".to_string()));
        assert!(tables.contains(&"sync_events".to_string()));
        assert!(tables.contains(&"rule_triggers".to_string()));
        assert!(tables.contains(&"schema_version".to_string()));
    }

    #[test]
    fn test_schema_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        // Running again should be a no-op
        init_schema(&conn).unwrap();

        let version: i32 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn test_schema_v2_migration() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();

        // Verify rule_triggers table exists and is functional
        conn.execute(
            "INSERT INTO rule_triggers (rule_name, outcome) VALUES ('test-rule', 'pass')",
            [],
        )
        .unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rule_triggers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Verify idempotent: running init_schema again should not fail
        init_schema(&conn).unwrap();

        // Data should still be there
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM rule_triggers", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    /// RT-45: the latest recorded version is the number of migrations.
    #[test]
    fn schema_version_matches_the_migration_list() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let version: i32 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert_eq!(SCHEMA_VERSION as usize, MIGRATIONS.len());
    }

    /// RT-43: two processes opening a fresh database both saw version 0 and
    /// both inserted version 1; the second failed on the primary key.
    #[test]
    fn concurrent_initialisation_of_a_fresh_database_succeeds() {
        for _ in 0..10 {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("fresh.db");
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(6));
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    let path = path.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        let conn = Connection::open(&path).unwrap();
                        barrier.wait();
                        init_schema(&conn)
                    })
                })
                .collect();
            for handle in handles {
                handle
                    .join()
                    .unwrap()
                    .expect("init_schema under contention");
            }
        }
    }
}
