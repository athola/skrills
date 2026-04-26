//! Validation cache for offline/cached mode.
//!
//! Stores last-known-good validation results in SQLite so that
//! `skrills validate` and `skrills analyze` can serve results
//! without network access.

use crate::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A cached validation result with timestamp.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CachedValidation {
    /// Path to the skill file.
    pub skill_path: String,
    /// Content hash at time of validation.
    pub content_hash: String,
    /// Serialized validation result (JSON).
    pub result_json: String,
    /// Unix timestamp when cached.
    pub cached_at: i64,
}

/// Staleness categories for cached data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    /// Less than 1 hour old.
    Fresh,
    /// 1-24 hours old.
    Recent,
    /// 1-7 days old.
    Aging,
    /// More than 7 days old.
    Stale,
}

impl std::fmt::Display for Staleness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Staleness::Fresh => write!(f, "fresh"),
            Staleness::Recent => write!(f, "recent"),
            Staleness::Aging => write!(f, "aging"),
            Staleness::Stale => write!(f, "stale"),
        }
    }
}

/// Determine the staleness of cached data based on age.
pub fn staleness_indicator(cached_at_unix: u64) -> Staleness {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    let age_secs = now.saturating_sub(cached_at_unix);
    let one_hour = 3600;
    let one_day = 86400;
    let one_week = 604800;

    if age_secs < one_hour {
        Staleness::Fresh
    } else if age_secs < one_day {
        Staleness::Recent
    } else if age_secs < one_week {
        Staleness::Aging
    } else {
        Staleness::Stale
    }
}

/// Human-readable age string from a unix timestamp.
pub fn human_age(cached_at_unix: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();

    let age_secs = now.saturating_sub(cached_at_unix);
    let minutes = age_secs / 60;
    let hours = age_secs / 3600;
    let days = age_secs / 86400;

    if age_secs < 60 {
        "just now".to_string()
    } else if minutes < 60 {
        format!(
            "{} minute{} ago",
            minutes,
            if minutes == 1 { "" } else { "s" }
        )
    } else if hours < 24 {
        format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" })
    } else {
        format!("{} day{} ago", days, if days == 1 { "" } else { "s" })
    }
}

/// Compute a SHA-256 hash of content for cache key purposes.
pub fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    hex_encode(&result)
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Cache schema SQL.
const CACHE_SCHEMA: &str = r"
CREATE TABLE IF NOT EXISTS validation_cache (
    skill_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    result_json TEXT NOT NULL,
    cached_at INTEGER NOT NULL,
    PRIMARY KEY (skill_path, content_hash)
);

CREATE INDEX IF NOT EXISTS idx_cache_path ON validation_cache(skill_path);
CREATE INDEX IF NOT EXISTS idx_cache_time ON validation_cache(cached_at);
";

/// SQLite-backed validation cache.
pub struct ValidationCache {
    conn: Connection,
}

impl ValidationCache {
    /// Open an in-memory cache (useful for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(CACHE_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Open a persistent cache at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(CACHE_SCHEMA)?;
        Ok(Self { conn })
    }

    /// Open the cache at the default path (`~/.skrills/validation_cache.db`).
    pub fn open_default() -> Result<Self> {
        let path = Self::default_path()?;
        Self::open(&path)
    }

    /// Get the default cache database path.
    pub fn default_path() -> Result<PathBuf> {
        let home = crate::home_dir()?;
        Ok(home.join(".skrills").join("validation_cache.db"))
    }

    /// Store a validation result in the cache.
    pub fn store_result(&self, skill_path: &str, hash: &str, result_json: &str) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs() as i64;

        self.conn.execute(
            "INSERT OR REPLACE INTO validation_cache (skill_path, content_hash, result_json, cached_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![skill_path, hash, result_json, now],
        )?;
        Ok(())
    }

    /// Retrieve a cached result by skill path and content hash.
    ///
    /// Returns `None` if no matching entry exists.
    pub fn get_cached_result(
        &self,
        skill_path: &str,
        hash: &str,
    ) -> Result<Option<CachedValidation>> {
        let mut stmt = self.conn.prepare(
            "SELECT skill_path, content_hash, result_json, cached_at FROM validation_cache WHERE skill_path = ?1 AND content_hash = ?2",
        )?;

        let result = stmt.query_row(rusqlite::params![skill_path, hash], |row| {
            Ok(CachedValidation {
                skill_path: row.get(0)?,
                content_hash: row.get(1)?,
                result_json: row.get(2)?,
                cached_at: row.get(3)?,
            })
        });

        match result {
            Ok(cached) => Ok(Some(cached)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get the most recent cached result for a skill path, regardless of hash.
    ///
    /// Useful for serving stale cached data when offline and content has changed.
    pub fn get_latest_for_path(&self, skill_path: &str) -> Result<Option<CachedValidation>> {
        let mut stmt = self.conn.prepare(
            "SELECT skill_path, content_hash, result_json, cached_at FROM validation_cache WHERE skill_path = ?1 ORDER BY cached_at DESC LIMIT 1",
        )?;

        let result = stmt.query_row(rusqlite::params![skill_path], |row| {
            Ok(CachedValidation {
                skill_path: row.get(0)?,
                content_hash: row.get(1)?,
                result_json: row.get(2)?,
                cached_at: row.get(3)?,
            })
        });

        match result {
            Ok(cached) => Ok(Some(cached)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Remove entries older than the given duration.
    pub fn cleanup_older_than(&self, max_age: Duration) -> Result<usize> {
        let cutoff = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs()
            .saturating_sub(max_age.as_secs()) as i64;

        let deleted = self.conn.execute(
            "DELETE FROM validation_cache WHERE cached_at < ?1",
            rusqlite::params![cutoff],
        )?;
        Ok(deleted)
    }

    /// Count total entries in the cache.
    pub fn entry_count(&self) -> Result<usize> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM validation_cache", [], |row| {
                    row.get(0)
                })?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_deterministic() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        assert_eq!(h1, h2);
        assert!(!h1.is_empty());
    }

    #[test]
    fn content_hash_differs_for_different_content() {
        let h1 = content_hash("hello");
        let h2 = content_hash("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn staleness_indicator_fresh() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(staleness_indicator(now), Staleness::Fresh);
        assert_eq!(staleness_indicator(now - 1800), Staleness::Fresh); // 30 min
    }

    #[test]
    fn staleness_indicator_recent() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(staleness_indicator(now - 7200), Staleness::Recent); // 2 hours
    }

    #[test]
    fn staleness_indicator_aging() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(staleness_indicator(now - 172800), Staleness::Aging); // 2 days
    }

    #[test]
    fn staleness_indicator_stale() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(staleness_indicator(now - 864000), Staleness::Stale); // 10 days
    }

    #[test]
    fn staleness_display() {
        assert_eq!(Staleness::Fresh.to_string(), "fresh");
        assert_eq!(Staleness::Recent.to_string(), "recent");
        assert_eq!(Staleness::Aging.to_string(), "aging");
        assert_eq!(Staleness::Stale.to_string(), "stale");
    }

    #[test]
    fn human_age_just_now() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(human_age(now), "just now");
    }

    #[test]
    fn human_age_minutes() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(human_age(now - 300), "5 minutes ago");
        assert_eq!(human_age(now - 60), "1 minute ago");
    }

    #[test]
    fn human_age_hours() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(human_age(now - 3600), "1 hour ago");
        assert_eq!(human_age(now - 7200), "2 hours ago");
    }

    #[test]
    fn human_age_days() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(human_age(now - 86400), "1 day ago");
        assert_eq!(human_age(now - 259200), "3 days ago");
    }

    // === ValidationCache tests ===

    #[test]
    fn cache_in_memory_creation() {
        let cache = ValidationCache::in_memory().unwrap();
        assert_eq!(cache.entry_count().unwrap(), 0);
    }

    #[test]
    fn cache_store_and_retrieve() {
        let cache = ValidationCache::in_memory().unwrap();

        cache
            .store_result("/path/to/skill.md", "abc123", r#"{"valid": true}"#)
            .unwrap();

        let result = cache
            .get_cached_result("/path/to/skill.md", "abc123")
            .unwrap();
        assert!(result.is_some());

        let cached = result.unwrap();
        assert_eq!(cached.skill_path, "/path/to/skill.md");
        assert_eq!(cached.content_hash, "abc123");
        assert_eq!(cached.result_json, r#"{"valid": true}"#);
        assert!(cached.cached_at > 0);
    }

    #[test]
    fn cache_miss_returns_none() {
        let cache = ValidationCache::in_memory().unwrap();

        let result = cache.get_cached_result("/nonexistent", "hash").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn cache_different_hash_returns_none() {
        let cache = ValidationCache::in_memory().unwrap();

        cache
            .store_result("/path/to/skill.md", "hash1", r#"{"v": 1}"#)
            .unwrap();

        let result = cache
            .get_cached_result("/path/to/skill.md", "hash2")
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn cache_upsert_on_same_key() {
        let cache = ValidationCache::in_memory().unwrap();

        cache
            .store_result("/path/to/skill.md", "hash1", r#"{"v": 1}"#)
            .unwrap();
        cache
            .store_result("/path/to/skill.md", "hash1", r#"{"v": 2}"#)
            .unwrap();

        let result = cache
            .get_cached_result("/path/to/skill.md", "hash1")
            .unwrap()
            .unwrap();
        assert_eq!(result.result_json, r#"{"v": 2}"#);
        assert_eq!(cache.entry_count().unwrap(), 1);
    }

    #[test]
    fn cache_get_latest_for_path() {
        let cache = ValidationCache::in_memory().unwrap();

        // Insert with explicit timestamps to ensure deterministic ordering
        cache.conn.execute(
            "INSERT INTO validation_cache (skill_path, content_hash, result_json, cached_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["/path/to/skill.md", "hash_old", r#"{"v": 1}"#, 1000i64],
        ).unwrap();
        cache.conn.execute(
            "INSERT INTO validation_cache (skill_path, content_hash, result_json, cached_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["/path/to/skill.md", "hash_new", r#"{"v": 2}"#, 2000i64],
        ).unwrap();

        let result = cache.get_latest_for_path("/path/to/skill.md").unwrap();
        assert!(result.is_some());
        // Should be the most recent entry (cached_at = 2000)
        let cached = result.unwrap();
        assert_eq!(cached.content_hash, "hash_new");
    }

    #[test]
    fn cache_get_latest_for_path_miss() {
        let cache = ValidationCache::in_memory().unwrap();
        let result = cache.get_latest_for_path("/nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn cache_entry_count() {
        let cache = ValidationCache::in_memory().unwrap();
        assert_eq!(cache.entry_count().unwrap(), 0);

        cache.store_result("a", "h1", "{}").unwrap();
        assert_eq!(cache.entry_count().unwrap(), 1);

        cache.store_result("b", "h2", "{}").unwrap();
        assert_eq!(cache.entry_count().unwrap(), 2);

        // Same key = upsert, count stays same
        cache.store_result("a", "h1", "{}").unwrap();
        assert_eq!(cache.entry_count().unwrap(), 2);
    }

    #[test]
    fn cache_cleanup_older_than() {
        let cache = ValidationCache::in_memory().unwrap();

        // Insert an entry with a very old timestamp by going through SQL directly
        cache
            .conn
            .execute(
                "INSERT INTO validation_cache (skill_path, content_hash, result_json, cached_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["old_skill", "hash", "{}", 1000i64],
            )
            .unwrap();

        // Insert a current entry
        cache.store_result("new_skill", "hash", "{}").unwrap();

        assert_eq!(cache.entry_count().unwrap(), 2);

        // Cleanup entries older than 1 day
        let deleted = cache
            .cleanup_older_than(Duration::from_secs(86400))
            .unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(cache.entry_count().unwrap(), 1);

        // The new entry should still exist
        let remaining = cache.get_cached_result("new_skill", "hash").unwrap();
        assert!(remaining.is_some());

        // The old entry should be gone
        let gone = cache.get_cached_result("old_skill", "hash").unwrap();
        assert!(gone.is_none());
    }

    #[test]
    fn cache_persistent_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_cache.db");

        // Store data
        {
            let cache = ValidationCache::open(&db_path).unwrap();
            cache
                .store_result("skill", "hash", r#"{"ok": true}"#)
                .unwrap();
        }

        // Reopen and verify data persists
        {
            let cache = ValidationCache::open(&db_path).unwrap();
            let result = cache.get_cached_result("skill", "hash").unwrap();
            assert!(result.is_some());
            assert_eq!(result.unwrap().result_json, r#"{"ok": true}"#);
        }
    }
}
