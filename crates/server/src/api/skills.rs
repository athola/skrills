//! Skills API endpoints.

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Serialize;
use std::sync::Arc;

use skrills_discovery::{discover_skills, SkillMeta, SkillRoot, SkillSource};

/// Shared state for API handlers.
#[derive(Clone)]
pub struct ApiState {
    /// Skill directories to scan.
    pub skill_dirs: Vec<std::path::PathBuf>,
}

/// Skill info for API response.
#[derive(Debug, Serialize)]
pub struct SkillResponse {
    pub name: String,
    pub path: String,
    pub source: String,
    pub description: Option<String>,
    pub hash: Option<String>,
}

impl From<SkillMeta> for SkillResponse {
    fn from(meta: SkillMeta) -> Self {
        Self {
            name: meta.name,
            path: meta.path.display().to_string(),
            source: format!("{:?}", meta.source),
            description: meta.description,
            hash: Some(meta.hash),
        }
    }
}

/// List all discovered skills.
async fn list_skills(State(state): State<Arc<ApiState>>) -> Json<Vec<SkillResponse>> {
    let roots: Vec<SkillRoot> = if state.skill_dirs.is_empty() {
        let mut roots = Vec::new();
        if let Some(home) = dirs::home_dir() {
            let claude_dir = home.join(".claude").join("skills");
            if claude_dir.exists() {
                roots.push(SkillRoot {
                    root: claude_dir,
                    source: SkillSource::Claude,
                });
            }
            let codex_dir = home.join(".codex").join("skills");
            if codex_dir.exists() {
                roots.push(SkillRoot {
                    root: codex_dir,
                    source: SkillSource::Codex,
                });
            }
        }
        roots
    } else {
        state
            .skill_dirs
            .iter()
            .map(|p| SkillRoot {
                root: p.clone(),
                source: SkillSource::Codex,
            })
            .collect()
    };

    let skills = discover_skills(&roots, None).unwrap_or_default();
    let responses: Vec<SkillResponse> = skills.into_iter().map(Into::into).collect();
    Json(responses)
}

/// Get a specific skill by name.
async fn get_skill(
    State(state): State<Arc<ApiState>>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<Json<SkillResponse>, StatusCode> {
    let roots: Vec<SkillRoot> = if state.skill_dirs.is_empty() {
        let mut roots = Vec::new();
        if let Some(home) = dirs::home_dir() {
            roots.push(SkillRoot {
                root: home.join(".claude").join("skills"),
                source: SkillSource::Claude,
            });
            roots.push(SkillRoot {
                root: home.join(".codex").join("skills"),
                source: SkillSource::Codex,
            });
        }
        roots
    } else {
        state
            .skill_dirs
            .iter()
            .map(|p| SkillRoot {
                root: p.clone(),
                source: SkillSource::Codex,
            })
            .collect()
    };

    let skills = discover_skills(&roots, None).unwrap_or_default();
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
        .route("/api/skills/:name", get(get_skill))
        .with_state(state)
}
