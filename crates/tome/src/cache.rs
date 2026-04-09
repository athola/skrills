//! SQLite-based response cache for API results and PDF storage.

use crate::{TomeError, TomeResult};
use rusqlite::Connection;
use std::path::PathBuf;

/// Cache for API responses and downloaded PDFs.
pub struct ResearchCache {
    conn: std::sync::Mutex<Connection>,
    pdf_dir: PathBuf,
}

impl ResearchCache {
    /// Opens or creates the cache database at the default location.
    pub fn open() -> TomeResult<Self> {
        let cache_dir = Self::cache_dir()?;
        let db_path = cache_dir.join("research.db");
        let pdf_dir = cache_dir.join("pdfs");
        std::fs::create_dir_all(&pdf_dir)?;

        let conn = Connection::open(&db_path)?;
        let cache = Self {
            conn: std::sync::Mutex::new(conn),
            pdf_dir,
        };
        cache.init_schema()?;
        Ok(cache)
    }

    /// Opens a cache with a custom path (for testing).
    pub fn open_at(db_path: &std::path::Path, pdf_dir: PathBuf) -> TomeResult<Self> {
        std::fs::create_dir_all(&pdf_dir)?;
        let conn = Connection::open(db_path)?;
        let cache = Self {
            conn: std::sync::Mutex::new(conn),
            pdf_dir,
        };
        cache.init_schema()?;
        Ok(cache)
    }

    fn init_schema(&self) -> TomeResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS api_cache (
                cache_key TEXT PRIMARY KEY,
                api TEXT NOT NULL,
                query TEXT NOT NULL,
                response_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                expires_at TEXT
            );

            CREATE TABLE IF NOT EXISTS research_sessions (
                id TEXT PRIMARY KEY,
                query TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                paper_count INTEGER NOT NULL DEFAULT 0,
                discussion_count INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS pdf_files (
                doi TEXT PRIMARY KEY,
                local_path TEXT NOT NULL,
                url TEXT,
                downloaded_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_cache_api ON api_cache(api);
            CREATE INDEX IF NOT EXISTS idx_cache_expires ON api_cache(expires_at);
            CREATE INDEX IF NOT EXISTS idx_sessions_created ON research_sessions(created_at);",
        )?;
        Ok(())
    }

    /// Get a cached API response by key.
    pub fn get(&self, cache_key: &str) -> TomeResult<Option<String>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let result = conn.query_row(
            "SELECT response_json FROM api_cache
             WHERE cache_key = ?1
             AND (expires_at IS NULL OR expires_at > datetime('now'))",
            [cache_key],
            |row| row.get(0),
        );
        match result {
            Ok(json) => Ok(Some(json)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(TomeError::Cache(e)),
        }
    }

    /// Store an API response in the cache.
    pub fn put(
        &self,
        cache_key: &str,
        api: &str,
        query: &str,
        response_json: &str,
        ttl_hours: Option<u32>,
    ) -> TomeResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(h) = ttl_hours {
            // Compute expiry in Rust and pass as a bound parameter to avoid SQL interpolation.
            let expires_at = format!("+{h} hours");
            conn.execute(
                "INSERT OR REPLACE INTO api_cache (cache_key, api, query, response_json, expires_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now', ?5))",
                rusqlite::params![cache_key, api, query, response_json, expires_at],
            )?;
        } else {
            conn.execute(
                "INSERT OR REPLACE INTO api_cache (cache_key, api, query, response_json)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![cache_key, api, query, response_json],
            )?;
        }
        Ok(())
    }

    /// Returns the PDF directory path.
    pub fn pdf_dir(&self) -> &std::path::Path {
        &self.pdf_dir
    }

    /// Returns the canonical skrills-tome cache directory, creating it if needed.
    ///
    /// Other modules (knowledge graph, citations) should use this instead of
    /// duplicating the path logic.
    pub fn cache_dir() -> TomeResult<std::path::PathBuf> {
        let base = dirs::cache_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".cache")))
            .ok_or_else(|| {
                TomeError::Other("cannot determine cache directory: HOME is unset".into())
            })?;
        let dir = base.join("skrills-tome");
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cache() -> (ResearchCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let pdfs = dir.path().join("pdfs");
        let cache = ResearchCache::open_at(&db, pdfs).unwrap();
        (cache, dir)
    }

    #[test]
    fn cache_miss_returns_none() {
        let (cache, _dir) = test_cache();
        assert!(cache.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn cache_put_and_get() {
        let (cache, _dir) = test_cache();
        cache
            .put("key1", "test_api", "test query", r#"{"results":[]}"#, None)
            .unwrap();
        let result = cache.get("key1").unwrap();
        assert_eq!(result, Some(r#"{"results":[]}"#.to_string()));
    }

    #[test]
    fn cache_overwrite() {
        let (cache, _dir) = test_cache();
        cache.put("key1", "api", "q", "v1", None).unwrap();
        cache.put("key1", "api", "q", "v2", None).unwrap();
        assert_eq!(cache.get("key1").unwrap(), Some("v2".to_string()));
    }

    /// GIVEN a valid HOME directory
    /// WHEN ResearchCache::cache_dir() is called
    /// THEN it returns a path ending in "skrills-tome" and creates the directory
    #[test]
    fn cache_dir_creates_directory() {
        let temp = tempfile::tempdir().unwrap();
        let prev = std::env::var("HOME").ok();

        // Run inside catch_unwind so HOME is always restored even on panic
        let result = std::panic::catch_unwind(|| {
            std::env::set_var("HOME", temp.path());
            ResearchCache::cache_dir()
        });

        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }

        let dir = result.expect("test panicked").unwrap();
        assert!(dir.ends_with("skrills-tome"));
        assert!(dir.exists(), "cache_dir should create the directory");
    }
}
