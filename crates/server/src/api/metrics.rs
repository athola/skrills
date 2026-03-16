//! Metrics API endpoints.
//!
//! REST API for skill usage metrics and statistics.
//!
//! ## Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/api/metrics/events` | Get recent metric events (max 100) |
//! | GET | `/api/metrics/skills/:skill` | Get stats for a specific skill |
//! | GET | `/api/metrics/analytics` | Get overall analytics summary |
//! | GET | `/api/metrics/analytics/top` | Get top skills by invocation count |
//! | GET | `/api/metrics/validation/summary` | Get validation summary across all skills |
//! | GET | `/api/metrics/validation/:skill` | Get validation history for a skill |
//! | GET | `/api/metrics/sync` | Get recent sync event history |
//! | GET | `/api/metrics/sync/summary` | Get sync summary statistics |
//! | GET | `/api/metrics/rules/analytics` | Get overall rule analytics summary |
//! | GET | `/api/metrics/rules/top` | Get top rules by trigger count |
//! | GET | `/api/metrics/rules/:rule` | Get effectiveness for a specific rule |
//!
//! ## Response Format
//!
//! All endpoints return JSON. Errors return appropriate HTTP status codes.

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;

use skrills_metrics::{
    AnalyticsSummary, MetricEvent, MetricsCollector, RuleAnalyticsSummary, RuleEffectiveness,
    SkillStats, SyncDetail, SyncSummary, TopSkill, ValidationDetail, ValidationSummary,
};

/// Metrics API state.
#[derive(Clone)]
pub struct MetricsState {
    /// Metrics collector.
    pub collector: Arc<MetricsCollector>,
}

/// Run a blocking metrics collector call on the Tokio blocking thread pool.
async fn blocking_query<T, F>(collector: &Arc<MetricsCollector>, f: F) -> Result<T, StatusCode>
where
    T: Send + 'static,
    F: FnOnce(&MetricsCollector) -> skrills_metrics::Result<T> + Send + 'static,
{
    let collector = collector.clone();
    tokio::task::spawn_blocking(move || f(&collector))
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Blocking task panicked");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map_err(|e| {
            tracing::warn!(error = %e, "Metrics query failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

/// Recent events response.
#[derive(Debug, Serialize)]
pub struct RecentEventsResponse {
    /// Array of recent metric events (max 100).
    pub events: Vec<MetricEvent>,
}

/// Stats response for a skill.
#[derive(Debug, Serialize)]
pub struct StatsResponse {
    /// Skill name.
    pub skill: String,
    /// Total number of invocations.
    pub total_invocations: u64,
    /// Number of successful invocations.
    pub successful_invocations: u64,
    /// Number of failed invocations.
    pub failed_invocations: u64,
    /// Average execution duration in milliseconds.
    pub avg_duration_ms: f64,
    /// Total tokens consumed.
    pub total_tokens: u64,
}

impl StatsResponse {
    fn from_stats(skill: String, stats: SkillStats) -> Self {
        Self {
            skill,
            total_invocations: stats.total_invocations(),
            successful_invocations: stats.successful_invocations,
            failed_invocations: stats.failed_invocations,
            avg_duration_ms: stats.avg_duration_ms,
            total_tokens: stats.total_tokens,
        }
    }
}

/// Get recent metric events.
async fn get_recent_events(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<RecentEventsResponse>, StatusCode> {
    let events = blocking_query(&state.collector, |c| c.get_recent_events(100)).await?;
    Ok(Json(RecentEventsResponse { events }))
}

/// Get stats for a specific skill.
async fn get_skill_stats(
    State(state): State<Arc<MetricsState>>,
    axum::extract::Path(skill): axum::extract::Path<String>,
) -> Result<Json<StatsResponse>, StatusCode> {
    let skill_name = skill.clone();
    let stats = blocking_query(&state.collector, move |c| c.get_skill_stats(&skill_name)).await?;
    Ok(Json(StatsResponse::from_stats(skill, stats)))
}

/// Response wrapper for analytics summary.
#[derive(Debug, Serialize)]
pub struct AnalyticsSummaryResponse {
    /// The analytics summary data.
    #[serde(flatten)]
    pub summary: AnalyticsSummary,
}

/// Response wrapper for top skills.
#[derive(Debug, Serialize)]
pub struct TopSkillsResponse {
    /// Top skills by invocation count.
    pub skills: Vec<TopSkill>,
}

/// Default limit for top skills query.
const DEFAULT_TOP_SKILLS_LIMIT: usize = 10;

/// Get overall analytics summary.
async fn get_analytics_summary(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<AnalyticsSummaryResponse>, StatusCode> {
    let summary = blocking_query(&state.collector, |c| c.get_analytics_summary()).await?;
    Ok(Json(AnalyticsSummaryResponse { summary }))
}

/// Get top skills by invocation count.
async fn get_top_skills(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<TopSkillsResponse>, StatusCode> {
    let skills =
        blocking_query(&state.collector, |c| c.get_top_skills(DEFAULT_TOP_SKILLS_LIMIT)).await?;
    Ok(Json(TopSkillsResponse { skills }))
}

/// Response wrapper for validation history.
#[derive(Debug, Serialize)]
pub struct ValidationHistoryResponse {
    /// Skill name.
    pub skill: String,
    /// Validation history entries.
    pub history: Vec<ValidationDetail>,
}

/// Response wrapper for validation summary.
#[derive(Debug, Serialize)]
pub struct ValidationSummaryResponse {
    /// The validation summary data.
    #[serde(flatten)]
    pub summary: ValidationSummary,
}

/// Default limit for validation history query.
const DEFAULT_VALIDATION_HISTORY_LIMIT: usize = 20;

/// Get validation history for a specific skill.
async fn get_validation_history(
    State(state): State<Arc<MetricsState>>,
    axum::extract::Path(skill): axum::extract::Path<String>,
) -> Result<Json<ValidationHistoryResponse>, StatusCode> {
    let skill_name = skill.clone();
    let history = blocking_query(&state.collector, move |c| {
        c.get_validation_history(&skill_name, DEFAULT_VALIDATION_HISTORY_LIMIT)
    })
    .await?;
    Ok(Json(ValidationHistoryResponse { skill, history }))
}

/// Get validation summary across all skills.
async fn get_validation_summary(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<ValidationSummaryResponse>, StatusCode> {
    let summary = blocking_query(&state.collector, |c| c.get_validation_summary()).await?;
    Ok(Json(ValidationSummaryResponse { summary }))
}

/// Response wrapper for sync history.
#[derive(Debug, Serialize)]
pub struct SyncHistoryResponse {
    /// Recent sync events.
    pub events: Vec<SyncDetail>,
}

/// Response wrapper for sync summary.
#[derive(Debug, Serialize)]
pub struct SyncSummaryResponse {
    /// The sync summary data.
    #[serde(flatten)]
    pub summary: SyncSummary,
}

/// Default limit for sync history query.
const DEFAULT_SYNC_HISTORY_LIMIT: usize = 50;

/// Get recent sync event history.
async fn get_sync_history(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<SyncHistoryResponse>, StatusCode> {
    let events =
        blocking_query(&state.collector, |c| c.get_sync_history(DEFAULT_SYNC_HISTORY_LIMIT))
            .await?;
    Ok(Json(SyncHistoryResponse { events }))
}

/// Get sync summary statistics.
async fn get_sync_summary(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<SyncSummaryResponse>, StatusCode> {
    let summary = blocking_query(&state.collector, |c| c.get_sync_summary()).await?;
    Ok(Json(SyncSummaryResponse { summary }))
}

/// Response wrapper for rule analytics summary.
#[derive(Debug, Serialize)]
pub struct RuleAnalyticsSummaryResponse {
    /// The rule analytics summary data.
    #[serde(flatten)]
    pub summary: RuleAnalyticsSummary,
}

/// Response wrapper for top rules.
#[derive(Debug, Serialize)]
pub struct TopRulesResponse {
    /// Top rules by trigger count.
    pub rules: Vec<RuleEffectiveness>,
}

/// Response wrapper for rule effectiveness.
#[derive(Debug, Serialize)]
pub struct RuleEffectivenessResponse {
    /// The rule effectiveness data.
    #[serde(flatten)]
    pub effectiveness: RuleEffectiveness,
}

/// Default limit for top rules query.
const DEFAULT_TOP_RULES_LIMIT: usize = 10;

/// Get overall rule analytics summary.
async fn get_rule_analytics_summary(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<RuleAnalyticsSummaryResponse>, StatusCode> {
    let summary = blocking_query(&state.collector, |c| c.get_rule_analytics_summary()).await?;
    Ok(Json(RuleAnalyticsSummaryResponse { summary }))
}

/// Get top rules by trigger count.
async fn get_top_rules(
    State(state): State<Arc<MetricsState>>,
) -> Result<Json<TopRulesResponse>, StatusCode> {
    let rules =
        blocking_query(&state.collector, |c| c.get_top_rules(DEFAULT_TOP_RULES_LIMIT)).await?;
    Ok(Json(TopRulesResponse { rules }))
}

/// Get effectiveness for a specific rule.
async fn get_rule_effectiveness(
    State(state): State<Arc<MetricsState>>,
    axum::extract::Path(rule): axum::extract::Path<String>,
) -> Result<Json<RuleEffectivenessResponse>, StatusCode> {
    let rule_name = rule.clone();
    let effectiveness =
        blocking_query(&state.collector, move |c| c.get_rule_effectiveness(&rule_name)).await?;
    Ok(Json(RuleEffectivenessResponse { effectiveness }))
}

/// Create metrics API routes.
pub fn metrics_routes(state: Arc<MetricsState>) -> Router {
    Router::new()
        .route("/api/metrics/events", get(get_recent_events))
        .route("/api/metrics/skills/{skill}", get(get_skill_stats))
        .route("/api/metrics/analytics", get(get_analytics_summary))
        .route("/api/metrics/analytics/top", get(get_top_skills))
        .route(
            "/api/metrics/validation/summary",
            get(get_validation_summary),
        )
        .route(
            "/api/metrics/validation/{skill}",
            get(get_validation_history),
        )
        .route("/api/metrics/sync", get(get_sync_history))
        .route("/api/metrics/sync/summary", get(get_sync_summary))
        .route(
            "/api/metrics/rules/analytics",
            get(get_rule_analytics_summary),
        )
        .route("/api/metrics/rules/top", get(get_top_rules))
        .route("/api/metrics/rules/{rule}", get(get_rule_effectiveness))
        .with_state(state)
}
