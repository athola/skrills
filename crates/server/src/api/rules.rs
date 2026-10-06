//! Rules API endpoints.
//!
//! REST API for hookify rule discovery and management.
//!
//! ## Endpoints
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/api/rules` | List all discovered rules |
//! | GET | `/api/rules/:name` | Get a specific rule by name |

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use skrills_discovery::RuleMeta;

/// Rules API state.
#[derive(Clone)]
pub struct RulesState {
    /// Discovered rules.
    pub rules: Arc<Vec<RuleMeta>>,
}

/// Rule info for API response.
#[derive(Debug, Serialize, Deserialize)]
pub struct RuleResponse {
    /// Rule name.
    pub name: String,
    /// Path to the rule configuration file.
    pub path: String,
    /// Discovery source (e.g. "user", "project", "hookify").
    pub source: String,
    /// Rule category/trigger event.
    pub category: String,
    /// Whether the rule is currently enabled.
    pub enabled: bool,
    /// Optional description of the rule.
    pub description: Option<String>,
    /// Command or script the rule executes.
    pub command: Option<String>,
}

impl From<RuleMeta> for RuleResponse {
    fn from(meta: RuleMeta) -> Self {
        Self {
            name: meta.name,
            path: super::strip_home_prefix(&meta.path),
            source: meta.source,
            category: meta.category.to_string(),
            enabled: meta.enabled,
            description: meta.description,
            command: meta.command,
        }
    }
}

/// List all discovered rules.
async fn list_rules(State(state): State<Arc<RulesState>>) -> Json<Vec<RuleResponse>> {
    let rules: Vec<RuleResponse> = state.rules.iter().cloned().map(Into::into).collect();
    Json(rules)
}

/// Get a specific rule by name.
async fn get_rule(
    State(state): State<Arc<RulesState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<RuleResponse>, StatusCode> {
    state
        .rules
        .iter()
        .find(|r| r.name == name)
        .cloned()
        .map(|r| Json(r.into()))
        .ok_or(StatusCode::NOT_FOUND)
}

/// Create rules API routes.
pub fn rules_routes(state: Arc<RulesState>) -> Router {
    Router::new()
        .route("/api/rules", get(list_rules))
        .route("/api/rules/{name}", get(get_rule))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_discovery::RuleCategory;
    use std::path::PathBuf;

    fn make_test_rules() -> Vec<RuleMeta> {
        vec![
            RuleMeta {
                name: "pre-commit-lint".to_string(),
                path: PathBuf::from("/home/user/.claude/hooks/pre-commit-lint.json"),
                source: "user".to_string(),
                category: RuleCategory::PreCommit,
                enabled: true,
                description: Some("Runs linter before commit".to_string()),
                command: Some("cargo clippy".to_string()),
            },
            RuleMeta {
                name: "post-commit-notify".to_string(),
                path: PathBuf::from("/project/.claude/hooks/post-commit-notify.yaml"),
                source: "project".to_string(),
                category: RuleCategory::PostCommit,
                enabled: false,
                description: None,
                command: None,
            },
        ]
    }

    #[test]
    fn rules_state_holds_rules() {
        let state = RulesState {
            rules: Arc::new(make_test_rules()),
        };
        assert_eq!(state.rules.len(), 2);
    }

    #[test]
    fn rule_response_from_rule_meta() {
        let meta = RuleMeta {
            name: "test-rule".to_string(),
            path: PathBuf::from("/tmp/test-rule.json"),
            source: "user".to_string(),
            category: RuleCategory::PreCommit,
            enabled: true,
            description: Some("A test rule".to_string()),
            command: Some("echo test".to_string()),
        };
        let response: RuleResponse = meta.into();
        assert_eq!(response.name, "test-rule");
        assert_eq!(response.source, "user");
        assert_eq!(response.category, "pre-commit");
        assert!(response.enabled);
        assert_eq!(response.description, Some("A test rule".to_string()));
        assert_eq!(response.command, Some("echo test".to_string()));
    }

    #[test]
    fn rule_response_from_meta_with_none_fields() {
        let meta = RuleMeta {
            name: "basic".to_string(),
            path: PathBuf::from("/tmp/basic.json"),
            source: "project".to_string(),
            category: RuleCategory::Other("custom".to_string()),
            enabled: false,
            description: None,
            command: None,
        };
        let response: RuleResponse = meta.into();
        assert_eq!(response.name, "basic");
        assert_eq!(response.category, "custom");
        assert!(!response.enabled);
        assert!(response.description.is_none());
        assert!(response.command.is_none());
    }

    #[test]
    fn rule_response_serialization() {
        let response = RuleResponse {
            name: "test".to_string(),
            path: "/tmp/test.json".to_string(),
            source: "user".to_string(),
            category: "pre-commit".to_string(),
            enabled: true,
            description: Some("desc".to_string()),
            command: Some("cmd".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        let deserialized: RuleResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test");
        assert_eq!(deserialized.category, "pre-commit");
    }

    async fn get(app: Router, uri: &str) -> (StatusCode, serde_json::Value) {
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
    async fn rules_routes_list_and_lookup() {
        let app = rules_routes(Arc::new(RulesState {
            rules: Arc::new(make_test_rules()),
        }));

        let (status, body) = get(app.clone(), "/api/rules").await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<&str> = body
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["pre-commit-lint", "post-commit-notify"]);

        let (status, body) = get(app.clone(), "/api/rules/post-commit-notify").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["source"], "project");
        assert_eq!(body["enabled"], false);

        let (status, _) = get(app, "/api/rules/nope").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
