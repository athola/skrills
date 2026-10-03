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
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
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

/// Maximum number of items a client can request per page.
///
/// Requests with `limit` above this value are silently clamped to `MAX_LIMIT`.
/// This prevents clients from fetching the entire skill set in one request,
/// which could be expensive for large installations with thousands of skills.
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
        Self {
            name: meta.name,
            path: super::strip_home_prefix(&meta.path),
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
) -> Result<Json<PaginatedResponse<SkillResponse>>, StatusCode> {
    let skills = load_skills(&state).await?;

    let total = skills.len();
    let limit = params.limit.min(MAX_LIMIT);
    let items: Vec<SkillResponse> = skills
        .into_iter()
        .skip(params.offset)
        .take(limit)
        .map(Into::into)
        .collect();

    Ok(Json(PaginatedResponse {
        items,
        total,
        limit,
        offset: params.offset,
    }))
}

/// Return the cached skill list, rescanning on a miss.
///
/// A rescan walks the skill directories, so it runs on the blocking pool
/// rather than stalling an async worker; the write lock is taken there too,
/// which keeps concurrent misses from scanning twice.
async fn load_skills(state: &Arc<ApiState>) -> Result<Vec<SkillMeta>, StatusCode> {
    if let Some(cached) = state.cache.read().get_cached() {
        return Ok(cached);
    }
    let state = Arc::clone(state);
    tokio::task::spawn_blocking(move || {
        let roots = skill_roots_or_default(&state.skill_dirs);
        state.cache.write().get_or_refresh(&roots)
    })
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "skill discovery task panicked");
        StatusCode::INTERNAL_SERVER_ERROR
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
    let skills = load_skills(&state).await?;

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
        assert!(result.is_empty());
        assert!(
            cache.last_refresh.is_some(),
            "a successful empty scan still counts as a refresh"
        );
    }

    fn write_skill(root: &std::path::Path, name: &str) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {name} skill\n---\nbody\n"),
        )
        .unwrap();
    }

    async fn get_json(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
        use tower::ServiceExt;
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    #[tokio::test]
    async fn list_skills_clamps_limit_and_reports_total() {
        let temp = tempfile::tempdir().unwrap();
        for i in 0..3 {
            write_skill(temp.path(), &format!("skill-{i}"));
        }
        let app = skills_routes(Arc::new(ApiState::new(vec![temp.path().to_path_buf()])));

        let (status, body) = get_json(app.clone(), "/api/skills?limit=999").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["limit"], MAX_LIMIT);
        assert_eq!(body["total"], 3);
        assert_eq!(body["items"].as_array().unwrap().len(), 3);

        let (_, page) = get_json(app, "/api/skills?limit=1&offset=2").await;
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        assert_eq!(page["offset"], 2);
    }

    #[tokio::test]
    async fn get_skill_finds_by_name_and_404s_otherwise() {
        let temp = tempfile::tempdir().unwrap();
        write_skill(temp.path(), "alpha");
        let app = skills_routes(Arc::new(ApiState::new(vec![temp.path().to_path_buf()])));

        let (_, list) = get_json(app.clone(), "/api/skills").await;
        let name = list["items"][0]["name"].as_str().unwrap().to_string();
        assert!(name.starts_with("alpha"), "{name}");

        let (status, body) = get_json(app.clone(), &format!("/api/skills/{name}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["name"], name.as_str());
        assert_eq!(body["description"], "alpha skill");

        let (status, _) = get_json(app, "/api/skills/missing").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
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
    fn api_state_creates_with_default_ttl() {
        let state = ApiState::new(vec![]);
        let cache = state.cache.read();
        assert_eq!(cache.ttl, Duration::from_secs(30));
    }

    #[test]
    fn rwlock_survives_panic_in_writer() {
        let state = ApiState::new(vec![]);
        // parking_lot::RwLock does not poison, the lock is released on panic
        let cache = state.cache.clone();
        let _ = std::thread::spawn(move || {
            let _guard = cache.write();
            panic!("intentional panic");
        })
        .join();

        // Lock should still be usable (no poison)
        let guard = state.cache.read();
        assert!(guard.get_cached().is_none());
    }
}
