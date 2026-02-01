//! Metrics API endpoints.

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;

use skrills_metrics::{MetricsCollector, SkillStats};

/// Metrics API state.
#[derive(Clone)]
pub struct MetricsState {
    /// Metrics collector.
    pub collector: Arc<MetricsCollector>,
}

/// Recent events response.
#[derive(Debug, Serialize)]
pub struct RecentEventsResponse {
    pub events: Vec<serde_json::Value>,
}

/// Stats response for a skill.
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub skill: String,
    pub total_invocations: u64,
    pub successful_invocations: u64,
    pub failed_invocations: u64,
    pub avg_duration_ms: f64,
    pub total_tokens: u64,
}

impl StatsResponse {
    fn from_stats(skill: String, stats: SkillStats) -> Self {
        Self {
            skill,
            total_invocations: stats.total_invocations,
            successful_invocations: stats.successful_invocations,
            failed_invocations: stats.failed_invocations,
            avg_duration_ms: stats.avg_duration_ms,
            total_tokens: stats.total_tokens,
        }
    }
}

/// Get recent metric events.
async fn get_recent_events(State(state): State<Arc<MetricsState>>) -> Json<RecentEventsResponse> {
    let events = state
        .collector
        .get_recent_events(100)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .collect();
    Json(RecentEventsResponse { events })
}

/// Get stats for a specific skill.
async fn get_skill_stats(
    State(state): State<Arc<MetricsState>>,
    axum::extract::Path(skill): axum::extract::Path<String>,
) -> Result<Json<StatsResponse>, StatusCode> {
    state
        .collector
        .get_skill_stats(&skill)
        .map(|stats| Json(StatsResponse::from_stats(skill, stats)))
        .map_err(|_| StatusCode::NOT_FOUND)
}

/// Create metrics API routes.
pub fn metrics_routes(state: Arc<MetricsState>) -> Router {
    Router::new()
        .route("/api/metrics/events", get(get_recent_events))
        .route("/api/metrics/skills/:skill", get(get_skill_stats))
        .with_state(state)
}
