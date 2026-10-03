//! REST API endpoints for the skrills visualization dashboard.

#[cfg(feature = "http-transport")]
pub mod cold_window;
#[cfg(feature = "http-transport")]
pub mod mcp_servers;
#[cfg(feature = "http-transport")]
pub mod metrics;
#[cfg(feature = "http-transport")]
pub mod rules;
#[cfg(feature = "http-transport")]
pub mod skills;

#[cfg(feature = "http-transport")]
pub use cold_window::{cold_window_routes, ColdWindowDashboardState};
#[cfg(feature = "http-transport")]
pub use mcp_servers::mcp_servers_routes;
#[cfg(feature = "http-transport")]
pub use metrics::metrics_routes;
#[cfg(feature = "http-transport")]
pub use rules::rules_routes;
#[cfg(feature = "http-transport")]
pub use skills::skills_routes;

/// Replace the user's home directory prefix with `~` to avoid leaking absolute paths.
#[cfg(feature = "http-transport")]
pub(crate) fn strip_home_prefix(path: &std::path::Path) -> String {
    match dirs::home_dir() {
        Some(home) => replace_home(path, &home),
        None => path.display().to_string(),
    }
}

/// Render `path` with a leading `home` replaced by `~`, matching whole path
/// components so `/home/al` does not swallow part of `/home/alice`.
#[cfg(feature = "http-transport")]
fn replace_home(path: &std::path::Path, home: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(all(test, feature = "http-transport"))]
mod tests {
    use super::replace_home;
    use std::path::Path;

    #[test]
    fn home_prefix_matches_whole_components() {
        let home = Path::new("/home/al");
        assert_eq!(
            replace_home(Path::new("/home/alice/skills/x"), home),
            "/home/alice/skills/x"
        );
        assert_eq!(
            replace_home(Path::new("/home/al/skills/x"), home),
            format!("~{}skills/x", std::path::MAIN_SEPARATOR)
        );
        assert_eq!(replace_home(home, home), "~");
    }
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

/// Serve the dashboard script. It is a file rather than an inline block so
/// the Content-Security-Policy can refuse inline script.
#[cfg(feature = "http-transport")]
async fn dashboard_script_handler() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/javascript; charset=utf-8",
        )],
        crate::ui::DASHBOARD_JS,
    )
}

/// Create dashboard UI routes: the page at `/` and its script at
/// `/static/dashboard.js`.
#[cfg(feature = "http-transport")]
pub fn dashboard_routes() -> Router {
    Router::new()
        .route("/", get(dashboard_handler))
        .route("/static/dashboard.js", get(dashboard_script_handler))
}
