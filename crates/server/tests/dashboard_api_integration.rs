//! Integration tests for the web dashboard REST API.
//!
//! Tests all dashboard endpoints using Axum's built-in test utilities
//! (tower::ServiceExt::oneshot) without starting a real HTTP server.

#![cfg(feature = "http-transport")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use std::path::PathBuf;
use std::sync::Arc;
use tower::ServiceExt;

use skrills_server::api::dashboard_routes;
use skrills_server::api::metrics::{metrics_routes, MetricsState};
use skrills_server::api::rules::{rules_routes, RuleResponse, RulesState};
use skrills_server::api::skills::{skills_routes, ApiState, PaginatedResponse, SkillResponse};

/// Create a test skill directory with sample SKILL.md files.
fn create_test_skills(dir: &std::path::Path) {
    for name in &["commit", "review", "deploy"] {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!(
                "---\nname: {}\ndescription: Test skill for {}\n---\n\n# {}\n\nTest content.",
                name, name, name
            ),
        )
        .unwrap();
    }
}

/// Build the skills API router with a test skill directory.
fn skills_app(skill_dirs: Vec<PathBuf>) -> Router {
    let state = Arc::new(ApiState::new(skill_dirs));
    skills_routes(state)
}

/// Build the metrics API router.
fn metrics_app() -> Router {
    let collector = Arc::new(
        skrills_metrics::MetricsCollector::new().expect("Failed to create metrics collector"),
    );
    let state = Arc::new(MetricsState { collector });
    metrics_routes(state)
}

/// Helper to extract body as string from response.
async fn body_string(body: Body) -> String {
    let bytes = body.collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

// ── Dashboard HTML Tests ──

#[tokio::test]
async fn dashboard_returns_html() {
    let app = dashboard_routes();

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("<!DOCTYPE html>"),
        "Should return HTML document"
    );
    assert!(
        body.contains("Skrills Dashboard"),
        "Should contain dashboard title"
    );
    assert!(
        body.contains("skill-list"),
        "Should contain skill list element"
    );
    assert!(
        body.contains("activity-list"),
        "Should contain activity list element"
    );
    assert!(
        body.contains("metrics-content"),
        "Should contain metrics content element"
    );
}

#[tokio::test]
async fn dashboard_html_includes_css_link() {
    let app = dashboard_routes();

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = body_string(response.into_body()).await;

    assert!(
        body.contains(r#"href="/static/style.css""#),
        "Dashboard HTML should link to stylesheet"
    );
}

#[tokio::test]
async fn dashboard_html_includes_javascript() {
    let app = dashboard_routes();

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = body_string(response.into_body()).await;

    assert!(
        body.contains("fetch('/api/skills"),
        "Dashboard should include JS that fetches skills API"
    );
    assert!(
        body.contains("fetch('/api/metrics/events')"),
        "Dashboard should include JS that fetches metrics API"
    );
}

#[tokio::test]
async fn dashboard_html_has_panel_structure() {
    let app = dashboard_routes();

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = body_string(response.into_body()).await;

    assert!(body.contains("skills-panel"), "Should have skills panel");
    assert!(
        body.contains("activity-panel"),
        "Should have activity panel"
    );
    assert!(body.contains("metrics-panel"), "Should have metrics panel");
    assert!(body.contains("<header>"), "Should have header");
    assert!(body.contains("<footer>"), "Should have footer");
}

#[tokio::test]
async fn dashboard_html_has_sort_button() {
    let app = dashboard_routes();

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = body_string(response.into_body()).await;

    assert!(
        body.contains("sort-btn"),
        "Dashboard should contain sort button element"
    );
    assert!(
        body.contains("Sort: Discovery"),
        "Sort button should default to 'Sort: Discovery'"
    );
}

// ── Skills API Tests ──
// Use multi_thread flavor to avoid blocking the runtime with sync discover_skills

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_skills_returns_paginated_json() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    let req = Request::builder()
        .uri("/api/skills")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let result: PaginatedResponse<SkillResponse> =
        serde_json::from_str(&body).expect("Response should be valid JSON");

    assert_eq!(result.total, 3, "Should find 3 skills");
    assert_eq!(result.items.len(), 3, "Should return all 3 in items");
    assert_eq!(result.offset, 0, "Default offset should be 0");

    let names: Vec<&str> = result.items.iter().map(|s| s.name.as_str()).collect();
    // Discovery uses the frontmatter `name` field from SKILL.md
    assert!(
        names.iter().any(|n| n.contains("commit")),
        "Should contain a skill with 'commit' in name. Got: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n.contains("review")),
        "Should contain a skill with 'review' in name. Got: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n.contains("deploy")),
        "Should contain a skill with 'deploy' in name. Got: {:?}",
        names
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_skills_with_pagination() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    let req = Request::builder()
        .uri("/api/skills?limit=2&offset=0")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let result: PaginatedResponse<SkillResponse> = serde_json::from_str(&body).unwrap();

    assert_eq!(result.items.len(), 2, "Should return only 2 items");
    assert_eq!(result.total, 3, "Total should still be 3");
    assert_eq!(result.limit, 2, "Limit should be 2");
    assert_eq!(result.offset, 0, "Offset should be 0");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_skills_with_offset() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    let req = Request::builder()
        .uri("/api/skills?limit=50&offset=2")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = body_string(response.into_body()).await;
    let result: PaginatedResponse<SkillResponse> = serde_json::from_str(&body).unwrap();

    assert_eq!(result.items.len(), 1, "Should return 1 item after offset=2");
    assert_eq!(result.offset, 2, "Offset should be 2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_skills_empty_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    let req = Request::builder()
        .uri("/api/skills")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let result: PaginatedResponse<SkillResponse> = serde_json::from_str(&body).unwrap();

    assert_eq!(result.total, 0, "Empty dir should have 0 skills");
    assert!(result.items.is_empty(), "Items should be empty");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_skill_by_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    // Skill names from discovery include the path suffix (e.g. "commit/SKILL.md")
    // The wildcard route /api/skills/{*name} captures the full path
    let list_req = Request::builder()
        .uri("/api/skills")
        .body(Body::empty())
        .unwrap();
    let list_resp = app.clone().oneshot(list_req).await.unwrap();
    let list_body = body_string(list_resp.into_body()).await;
    let list_result: PaginatedResponse<SkillResponse> = serde_json::from_str(&list_body).unwrap();
    let first_name = list_result.items[0].name.clone();

    let req = Request::builder()
        .uri(format!("/api/skills/{}", first_name))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let skill: SkillResponse = serde_json::from_str(&body).unwrap();

    assert_eq!(skill.name, first_name);
    assert!(!skill.path.is_empty(), "Path should not be empty");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn get_skill_not_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    let req = Request::builder()
        .uri("/api/skills/nonexistent")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Should return 404 for nonexistent skill"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_response_has_expected_fields() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    let req = Request::builder()
        .uri("/api/skills")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = body_string(response.into_body()).await;

    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let first_item = &json["items"][0];

    assert!(first_item["name"].is_string(), "Should have name field");
    assert!(first_item["path"].is_string(), "Should have path field");
    assert!(first_item["source"].is_string(), "Should have source field");
}

// ── Metrics API Tests ──

#[tokio::test]
async fn get_recent_events_returns_json() {
    let app = metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/events")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(
        json["events"].is_array(),
        "Response should have events array"
    );
}

#[tokio::test]
async fn get_skill_stats_returns_json() {
    let collector = Arc::new(
        skrills_metrics::MetricsCollector::new().expect("Failed to create metrics collector"),
    );

    collector
        .record_skill_invocation("test-skill", 100, true, Some(50))
        .unwrap();

    let state = Arc::new(MetricsState { collector });
    let app = metrics_routes(state);

    let req = Request::builder()
        .uri("/api/metrics/skills/test-skill")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(json["skill"], "test-skill");
    assert_eq!(json["total_invocations"], 1);
    assert_eq!(json["successful_invocations"], 1);
    assert_eq!(json["failed_invocations"], 0);
}

// ── Static File Serving Tests ──

#[tokio::test]
async fn static_css_is_served() {
    let app = Router::new().route(
        "/static/style.css",
        axum::routing::get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "text/css")],
                include_str!("../static/style.css"),
            )
        }),
    );

    let req = Request::builder()
        .uri("/static/style.css")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(content_type, "text/css", "Content-Type should be text/css");

    let body = body_string(response.into_body()).await;
    assert!(
        body.contains("--bg-deep"),
        "CSS should contain CSS variables"
    );
    assert!(body.contains(".skill-item"), "CSS should style skill items");
    assert!(body.contains(".panel"), "CSS should style panels");
}

// ── Full Router Integration Tests ──

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_router_serves_dashboard_and_api() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());

    let api_state = Arc::new(ApiState::new(vec![tmp.path().to_path_buf()]));
    let collector = Arc::new(
        skrills_metrics::MetricsCollector::new().expect("Failed to create metrics collector"),
    );
    let metrics_state = Arc::new(MetricsState { collector });

    let static_router = Router::new().route(
        "/static/style.css",
        axum::routing::get(|| async {
            (
                [(axum::http::header::CONTENT_TYPE, "text/css")],
                include_str!("../static/style.css"),
            )
        }),
    );

    let app = Router::new()
        .merge(dashboard_routes())
        .merge(skills_routes(api_state))
        .merge(metrics_routes(metrics_state))
        .merge(static_router);

    // Test dashboard
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Test skills API
    let req = Request::builder()
        .uri("/api/skills")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_string(resp.into_body()).await;
    let result: PaginatedResponse<SkillResponse> = serde_json::from_str(&body).unwrap();
    assert_eq!(result.total, 3);

    // Test metrics events API
    let req = Request::builder()
        .uri("/api/metrics/events")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Test static CSS
    let req = Request::builder()
        .uri("/static/style.css")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── Pagination Edge Cases ──

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_limit_capped_at_max() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    let req = Request::builder()
        .uri("/api/skills?limit=999")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = body_string(response.into_body()).await;
    let result: PaginatedResponse<SkillResponse> = serde_json::from_str(&body).unwrap();

    assert!(
        result.limit <= 200,
        "Limit should be capped at MAX_LIMIT (200)"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pagination_offset_beyond_total() {
    let tmp = tempfile::TempDir::new().unwrap();
    create_test_skills(tmp.path());
    let app = skills_app(vec![tmp.path().to_path_buf()]);

    let req = Request::builder()
        .uri("/api/skills?offset=100")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = body_string(response.into_body()).await;
    let result: PaginatedResponse<SkillResponse> = serde_json::from_str(&body).unwrap();

    assert!(
        result.items.is_empty(),
        "Offset beyond total should return empty items"
    );
    assert_eq!(result.total, 3, "Total should still reflect actual count");
}

// ── Analytics & Validation & Sync & Rules Metrics API Tests ──

/// Build a metrics app with seed data for comprehensive testing.
fn seeded_metrics_app() -> Router {
    let collector = Arc::new(
        skrills_metrics::MetricsCollector::new().expect("Failed to create metrics collector"),
    );

    // Seed skill invocations
    collector
        .record_skill_invocation("alpha", 100, true, Some(50))
        .unwrap();
    collector
        .record_skill_invocation("alpha", 200, false, None)
        .unwrap();
    collector
        .record_skill_invocation("beta", 150, true, Some(30))
        .unwrap();

    // Seed validation runs
    collector
        .record_validation("alpha", &["lint", "test"], &["format"])
        .unwrap();
    collector
        .record_validation("beta", &["lint", "test", "format"], &[])
        .unwrap();

    // Seed sync events
    collector
        .record_sync_event(
            skrills_metrics::SyncOperation::Push,
            3,
            skrills_metrics::SyncStatus::Success,
        )
        .unwrap();
    collector
        .record_sync_event(
            skrills_metrics::SyncOperation::Pull,
            5,
            skrills_metrics::SyncStatus::Failed,
        )
        .unwrap();

    // Seed rule triggers
    collector
        .record_rule_trigger(
            "no-console",
            Some("lint"),
            Some("user"),
            Some(50),
            skrills_metrics::RuleOutcome::Pass,
            None,
        )
        .unwrap();
    collector
        .record_rule_trigger(
            "no-console",
            Some("lint"),
            Some("ci"),
            Some(30),
            skrills_metrics::RuleOutcome::Fail,
            None,
        )
        .unwrap();
    collector
        .record_rule_trigger(
            "require-tests",
            Some("quality"),
            Some("ci"),
            Some(80),
            skrills_metrics::RuleOutcome::Pass,
            None,
        )
        .unwrap();

    let state = Arc::new(MetricsState { collector });
    metrics_routes(state)
}

#[tokio::test]
async fn get_analytics_summary_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/analytics")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(json["total_invocations"], 3);
    assert_eq!(json["successful_invocations"], 2);
    assert_eq!(json["failed_invocations"], 1);
    assert_eq!(json["unique_skills"], 2);
}

#[tokio::test]
async fn get_top_skills_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/analytics/top")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(json["skills"].is_array());
    let skills = json["skills"].as_array().unwrap();
    assert!(!skills.is_empty(), "Should have at least one top skill");
    // "alpha" has 2 invocations, should be first
    assert_eq!(skills[0]["skill_name"], "alpha");
}

#[tokio::test]
async fn get_validation_summary_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/validation/summary")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(json["total_skills"], 2);
}

#[tokio::test]
async fn get_validation_history_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/validation/alpha")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(json["skill"], "alpha");
    assert!(json["history"].is_array());
    let history = json["history"].as_array().unwrap();
    assert_eq!(history.len(), 1, "Should have 1 validation entry for alpha");
}

#[tokio::test]
async fn get_sync_history_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/sync")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(json["events"].is_array());
    let events = json["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "Should have 2 sync events");
}

#[tokio::test]
async fn get_sync_summary_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/sync/summary")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(json["total_syncs"], 2);
    assert_eq!(json["successful_syncs"], 1);
    assert_eq!(json["failed_syncs"], 1);
}

#[tokio::test]
async fn get_rule_analytics_summary_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/rules/analytics")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(json["total_triggers"], 3);
    assert_eq!(json["unique_rules"], 2);
}

#[tokio::test]
async fn get_top_rules_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/rules/top")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert!(json["rules"].is_array());
    let rules = json["rules"].as_array().unwrap();
    assert!(!rules.is_empty());
    // "no-console" has 2 triggers, should be first
    assert_eq!(rules[0]["rule_name"], "no-console");
}

#[tokio::test]
async fn get_rule_effectiveness_returns_json() {
    let app = seeded_metrics_app();

    let req = Request::builder()
        .uri("/api/metrics/rules/no-console")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(json["rule_name"], "no-console");
    assert_eq!(json["total_triggers"], 2);
    assert_eq!(json["pass_count"], 1);
    assert_eq!(json["fail_count"], 1);
}

// ── Rules API Tests ──

fn rules_app() -> Router {
    use skrills_discovery::{RuleCategory, RuleMeta};
    use std::path::PathBuf;

    let rules = vec![
        RuleMeta {
            name: "pre-commit-lint".to_string(),
            path: PathBuf::from("/tmp/hooks/pre-commit-lint.json"),
            source: "user".to_string(),
            category: RuleCategory::PreCommit,
            enabled: true,
            description: Some("Runs linter before commit".to_string()),
            command: Some("cargo clippy".to_string()),
        },
        RuleMeta {
            name: "post-push-notify".to_string(),
            path: PathBuf::from("/tmp/hooks/post-push-notify.yaml"),
            source: "project".to_string(),
            category: RuleCategory::PostCommit,
            enabled: false,
            description: None,
            command: None,
        },
    ];

    let state = Arc::new(RulesState {
        rules: Arc::new(rules),
    });
    rules_routes(state)
}

#[tokio::test]
async fn list_rules_returns_json() {
    let app = rules_app();

    let req = Request::builder()
        .uri("/api/rules")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let rules: Vec<RuleResponse> = serde_json::from_str(&body).unwrap();

    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].name, "pre-commit-lint");
    assert!(rules[0].enabled);
    assert_eq!(rules[1].name, "post-push-notify");
    assert!(!rules[1].enabled);
}

#[tokio::test]
async fn get_rule_by_name_returns_json() {
    let app = rules_app();

    let req = Request::builder()
        .uri("/api/rules/pre-commit-lint")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_string(response.into_body()).await;
    let rule: RuleResponse = serde_json::from_str(&body).unwrap();

    assert_eq!(rule.name, "pre-commit-lint");
    assert_eq!(rule.source, "user");
    assert_eq!(rule.category, "pre-commit");
    assert!(rule.enabled);
    assert_eq!(rule.description, Some("Runs linter before commit".to_string()));
}

#[tokio::test]
async fn get_rule_not_found_returns_404() {
    let app = rules_app();

    let req = Request::builder()
        .uri("/api/rules/nonexistent")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Should return 404 for nonexistent rule"
    );
}
