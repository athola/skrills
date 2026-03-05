//! Database schema initialization.

use rusqlite::Connection;

use crate::Result;

/// SQL statements to create the metrics schema.
pub const SCHEMA: &str = r#"
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

/// Initialize the database schema.
pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
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
    }
}
