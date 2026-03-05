//! Database schema initialization and migrations.

use rusqlite::Connection;

use crate::Result;

/// Current schema version.
const SCHEMA_VERSION: i32 = 1;

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

/// Initialize the database schema with versioned migrations.
///
/// Creates a `schema_version` table to track the current version, then
/// applies any pending migrations in order.
pub fn init_schema(conn: &Connection) -> Result<()> {
    // Create version tracking table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER PRIMARY KEY
        )",
    )?;

    let current: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    // Apply migrations in order
    if current < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [1])?;
    }

    // Future migrations go here:
    // if current < 2 {
    //     conn.execute_batch(SCHEMA_V2)?;
    //     conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [2])?;
    // }

    debug_assert_eq!(SCHEMA_VERSION, 1, "update migrations when bumping SCHEMA_VERSION");

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
        assert_eq!(version, 1);
    }
}
