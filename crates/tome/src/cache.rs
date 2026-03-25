//! SQLite-based response cache for API results and PDF storage.

use crate::{TomeError, TomeResult};
use rusqlite::Connection;
use std::path::PathBuf;

/// Cache for API responses and downloaded PDFs.
pub struct ResearchCache {
    conn: Connection,
    pdf_dir: PathBuf,
}

impl ResearchCache {
    /// Opens or creates the cache database at the default location.
    pub fn open() -> TomeResult<Self> {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".cache"))
            .join("skrills-tome");
        std::fs::create_dir_all(&cache_dir)?;

        let db_path = cache_dir.join("research.db");
        let pdf_dir = cache_dir.join("pdfs");
        std::fs::create_dir_all(&pdf_dir)?;

        let conn = Connection::open(&db_path)?;
        let cache = Self { conn, pdf_dir };
        cache.init_schema()?;
        Ok(cache)
    }

    /// Opens a cache with a custom path (for testing).
    pub fn open_at(db_path: &std::path::Path, pdf_dir: PathBuf) -> TomeResult<Self> {
        std::fs::create_dir_all(&pdf_dir)?;
        let conn = Connection::open(db_path)?;
        let cache = Self { conn, pdf_dir };
        cache.init_schema()?;
        Ok(cache)
    }

    fn init_schema(&self) -> TomeResult<()> {
        self.conn.execute_batch(
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
        let result = self.conn.query_row(
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
        let expires = ttl_hours.map(|h| format!("datetime('now', '+{h} hours')"));
        if let Some(ref exp) = expires {
            self.conn.execute(
                &format!(
                    "INSERT OR REPLACE INTO api_cache (cache_key, api, query, response_json, expires_at)
                     VALUES (?1, ?2, ?3, ?4, {exp})"
                ),
                rusqlite::params![cache_key, api, query, response_json],
            )?;
        } else {
            self.conn.execute(
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_cache() -> ResearchCache {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let pdfs = dir.path().join("pdfs");
        // Leak the tempdir so it lives long enough
        let cache = ResearchCache::open_at(&db, pdfs).unwrap();
        std::mem::forget(dir);
        cache
    }

    #[test]
    fn cache_miss_returns_none() {
        let cache = test_cache();
        assert!(cache.get("nonexistent").unwrap().is_none());
    }

    #[test]
    fn cache_put_and_get() {
        let cache = test_cache();
        cache
            .put("key1", "test_api", "test query", r#"{"results":[]}"#, None)
            .unwrap();
        let result = cache.get("key1").unwrap();
        assert_eq!(result, Some(r#"{"results":[]}"#.to_string()));
    }

    #[test]
    fn cache_overwrite() {
        let cache = test_cache();
        cache.put("key1", "api", "q", "v1", None).unwrap();
        cache.put("key1", "api", "q", "v2", None).unwrap();
        assert_eq!(cache.get("key1").unwrap(), Some("v2".to_string()));
    }
}
