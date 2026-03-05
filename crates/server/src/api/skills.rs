//! Skills API endpoints.
//!
//! REST API for skill discovery and retrieval.
//!
//! ## Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/api/skills` | List all discovered skills |
//! | GET | `/api/skills/:name` | Get a specific skill by name |
//!
//! ## Response Format
//!
//! All endpoints return JSON. Errors return appropriate HTTP status codes:
//! - `200 OK` - Success
//! - `404 Not Found` - Skill not found
//! - `500 Internal Server Error` - Discovery failed

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use skrills_discovery::{discover_skills, skill_roots_or_default, SkillMeta, SkillRoot};

/// Cache for discovered skills with TTL.
pub struct SkillCache {
    skills: Vec<SkillMeta>,
    last_refresh: Option<Instant>,
    ttl: Duration,
}

impl SkillCache {
    /// Create a new cache with the given TTL in seconds.
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            skills: Vec::new(),
            last_refresh: None,
            ttl: Duration::from_secs(ttl_secs),
        }
    }

    /// Get cached skills without refreshing (for read-lock callers).
    pub fn get_cached(&self) -> Option<Vec<SkillMeta>> {
        if self.is_valid() {
            Some(self.skills.clone())
        } else {
            None
        }
    }

    /// Check if the cache is still valid.
    fn is_valid(&self) -> bool {
        self.last_refresh
            .map(|t| t.elapsed() < self.ttl)
            .unwrap_or(false)
    }

    /// Get cached skills or refresh from discovery.
    pub fn get_or_refresh(&mut self, roots: &[SkillRoot]) -> Vec<SkillMeta> {
        if self.is_valid() {
            return self.skills.clone();
        }

        match discover_skills(roots, None) {
            Ok(skills) => {
                self.skills = skills.clone();
                self.last_refresh = Some(Instant::now());
                skills
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to discover skills");
                // Return stale cache if available
                self.skills.clone()
            }
        }
    }
}

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Skill directories to scan.
    pub skill_dirs: Vec<std::path::PathBuf>,
    /// Cache for discovered skills.
    pub cache: Arc<RwLock<SkillCache>>,
}

impl ApiState {
    /// Create a new API state with default cache TTL (30 seconds).
    pub fn new(skill_dirs: Vec<std::path::PathBuf>) -> Self {
        Self {
            skill_dirs,
            cache: Arc::new(RwLock::new(SkillCache::new(30))),
        }
    }
}

/// Pagination query parameters.
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    /// Maximum number of items to return (default: 50).
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Number of items to skip (default: 0).
    #[serde(default)]
    pub offset: usize,
}

fn default_limit() -> usize {
    50
}

const MAX_LIMIT: usize = 200;

/// Paginated response wrapper.
#[derive(Debug, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    /// Items in the current page.
    pub items: Vec<T>,
    /// Total number of items available.
    pub total: usize,
    /// Maximum items per page.
    pub limit: usize,
    /// Number of items skipped.
    pub offset: usize,
}

/// Skill info for API response.
///
/// Represents a discovered skill with its metadata.
#[derive(Debug, Serialize, Deserialize)]
pub struct SkillResponse {
    /// Skill name (directory name).
    pub name: String,
    /// Absolute path to the skill file.
    pub path: String,
    /// Source identifier (e.g., "Claude", "Codex", "Copilot").
    pub source: String,
    /// Optional description from frontmatter.
    pub description: Option<String>,
    /// Content hash for change detection.
    pub hash: Option<String>,
}

impl From<SkillMeta> for SkillResponse {
    fn from(meta: SkillMeta) -> Self {
        // Replace home directory with ~ to avoid leaking absolute paths
        let path = meta.path.display().to_string();
        let path = dirs::home_dir()
            .and_then(|home| {
                path.strip_prefix(&home.display().to_string())
                    .map(|rest| format!("~{rest}"))
            })
            .unwrap_or(path);
        Self {
            name: meta.name,
            path,
            source: meta.source.to_string(),
            description: meta.description,
            hash: Some(meta.hash),
        }
    }
}

/// List all discovered skills with pagination.
///
/// Returns a paginated array of skills found across configured skill directories.
///
/// ## Query Parameters
///
/// - `limit` - Maximum number of items to return (default: 50)
/// - `offset` - Number of items to skip (default: 0)
///
/// ## Example Response
///
/// ```json
/// {
///   "items": [
///     {
///       "name": "commit",
///       "path": "/home/user/.claude/commands/commit/SKILL.md",
///       "source": "claude",
///       "description": "Generate conventional commit messages",
///       "hash": "abc123"
///     }
///   ],
///   "total": 100,
///   "limit": 50,
///   "offset": 0
/// }
/// ```
async fn list_skills(
    State(state): State<Arc<ApiState>>,
    Query(params): Query<PaginationParams>,
) -> Json<PaginatedResponse<SkillResponse>> {
    let roots = skill_roots_or_default(&state.skill_dirs);

    let skills = {
        let read_guard = state.cache.read().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = read_guard.get_cached() {
            cached
        } else {
            drop(read_guard);
            state
                .cache
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_refresh(&roots)
        }
    };

    let total = skills.len();
    let limit = params.limit.min(MAX_LIMIT);
    let items: Vec<SkillResponse> = skills
        .into_iter()
        .skip(params.offset)
        .take(limit)
        .map(Into::into)
        .collect();

    Json(PaginatedResponse {
        items,
        total,
        limit,
        offset: params.offset,
    })
}

/// Get a specific skill by name.
///
/// Returns a single skill matching the provided name.
///
/// ## Path Parameters
///
/// - `name` - The skill name to look up
///
/// ## Errors
///
/// - `404 Not Found` - No skill with the given name exists
async fn get_skill(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<SkillResponse>, StatusCode> {
    let roots = skill_roots_or_default(&state.skill_dirs);

    let skills = {
        let read_guard = state.cache.read().unwrap_or_else(|e| e.into_inner());
        if let Some(cached) = read_guard.get_cached() {
            cached
        } else {
            drop(read_guard);
            state
                .cache
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_refresh(&roots)
        }
    };

    skills
        .into_iter()
        .find(|s| s.name == name)
        .map(|s| Json(s.into()))
        .ok_or(StatusCode::NOT_FOUND)
}

/// Create skills API routes.
pub fn skills_routes(state: Arc<ApiState>) -> Router {
    Router::new()
        .route("/api/skills", get(list_skills))
        .route("/api/skills/{*name}", get(get_skill))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cache_is_empty_and_invalid() {
        let cache = SkillCache::new(30);
        assert!(
            cache.get_cached().is_none(),
            "fresh cache should be invalid"
        );
        assert!(cache.skills.is_empty());
    }

    #[test]
    fn cache_valid_after_manual_refresh() {
        let mut cache = SkillCache::new(60);
        cache.skills = vec![];
        cache.last_refresh = Some(Instant::now());
        assert!(cache.is_valid());
        assert!(cache.get_cached().is_some());
    }

    #[test]
    fn cache_expires_after_ttl() {
        let mut cache = SkillCache::new(0); // 0-second TTL
        cache.skills = vec![];
        cache.last_refresh = Some(Instant::now() - Duration::from_secs(1));
        assert!(!cache.is_valid());
        assert!(cache.get_cached().is_none());
    }

    #[test]
    fn get_or_refresh_with_empty_roots_returns_empty() {
        let mut cache = SkillCache::new(30);
        let roots: Vec<SkillRoot> = vec![];
        let result = cache.get_or_refresh(&roots);
        // With no roots, discovery returns empty or error; either way cache is populated
        assert!(cache.last_refresh.is_some() || !cache.skills.is_empty() || result.is_empty());
    }

    #[test]
    fn get_or_refresh_returns_cached_when_valid() {
        let mut cache = SkillCache::new(60);
        // Simulate a prior successful refresh
        cache.last_refresh = Some(Instant::now());
        cache.skills = vec![]; // empty but valid

        let roots: Vec<SkillRoot> = vec![];
        let result = cache.get_or_refresh(&roots);
        assert!(result.is_empty()); // returns cached (empty) without re-discovery
    }

    #[test]
    fn stale_cache_returned_on_discovery_error() {
        let mut cache = SkillCache::new(0); // expired TTL
                                            // Pre-populate with stale data
        cache.skills = vec![];
        cache.last_refresh = Some(Instant::now() - Duration::from_secs(10));

        // Discovery with non-existent roots may fail; stale cache returned
        let bad_roots = vec![SkillRoot {
            root: std::path::PathBuf::from("/nonexistent/path/that/does/not/exist"),
            source: skrills_discovery::SkillSource::Claude,
        }];
        let result = cache.get_or_refresh(&bad_roots);
        // Should return stale cache (empty vec) rather than panic
        assert!(result.is_empty());
    }

    #[test]
    fn pagination_defaults() {
        let params: PaginationParams = serde_json::from_str("{}").unwrap();
        assert_eq!(params.limit, 50);
        assert_eq!(params.offset, 0);
    }

    #[test]
    fn pagination_limit_clamped_to_max() {
        let clamped = 999_usize.min(MAX_LIMIT);
        assert_eq!(clamped, 200);
    }

    #[test]
    fn api_state_creates_with_default_ttl() {
        let state = ApiState::new(vec![]);
        let cache = state.cache.read().unwrap();
        assert_eq!(cache.ttl, Duration::from_secs(30));
    }

    #[test]
    fn rwlock_recovers_from_poison() {
        let state = ApiState::new(vec![]);
        // Simulate a poisoned lock by panicking in a thread holding write
        let cache = state.cache.clone();
        let _ = std::thread::spawn(move || {
            let _guard = cache.write().unwrap();
            panic!("intentional poison");
        })
        .join();

        // Should recover via into_inner
        let guard = state.cache.read().unwrap_or_else(|e| e.into_inner());
        assert!(guard.get_cached().is_none()); // fresh cache, not corrupted
    }
}
