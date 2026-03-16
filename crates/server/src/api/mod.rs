//! REST API endpoints for the skrills visualization dashboard.

#[cfg(feature = "http-transport")]
pub mod metrics;
#[cfg(feature = "http-transport")]
pub mod rules;
#[cfg(feature = "http-transport")]
pub mod skills;

#[cfg(feature = "http-transport")]
pub use metrics::metrics_routes;
#[cfg(feature = "http-transport")]
pub use rules::rules_routes;
#[cfg(feature = "http-transport")]
pub use skills::skills_routes;

/// Replace the user's home directory prefix with `~` to avoid leaking absolute paths.
#[cfg(feature = "http-transport")]
pub(crate) fn strip_home_prefix(path: &std::path::Path) -> String {
    let display = path.display().to_string();
    dirs::home_dir()
        .and_then(|home| {
            display
                .strip_prefix(&home.display().to_string())
                .map(|rest| format!("~{rest}"))
        })
        .unwrap_or(display)
}

#[cfg(feature = "http-transport")]
use axum::{
    response::{Html, IntoResponse},
    routing::get,
    Router,
};

/// Serve the Leptos dashboard HTML.
#[cfg(feature = "http-transport")]
async fn dashboard_handler() -> impl IntoResponse {
    Html(crate::ui::render_dashboard())
}

/// Create dashboard UI routes.
#[cfg(feature = "http-transport")]
pub fn dashboard_routes() -> Router {
    Router::new().route("/", get(dashboard_handler))
}
