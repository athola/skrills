//! Leptos components for the skrills dashboard.

use leptos::prelude::*;

/// Main dashboard application component.
#[component]
pub fn App() -> impl IntoView {
    view! {
        <!DOCTYPE html>
        <html lang="en">
            <head>
                <meta charset="UTF-8"/>
                <meta name="viewport" content="width=device-width, initial-scale=1.0"/>
                <title>"Skrills Dashboard"</title>
                <link rel="stylesheet" href="/static/style.css"/>
            </head>
            <body>
                <Dashboard/>
                // Loaded by URL so the CSP can forbid inline script.
                <script src="/static/dashboard.js"></script>
            </body>
        </html>
    }
}

/// Dashboard layout with skills panel, metrics panel, activity panel, and MCP servers panel.
#[component]
pub fn Dashboard() -> impl IntoView {
    view! {
        <header>
            <h1>"Skrills Dashboard"</h1>
            <div class="stats">
                <span>"Skills: "<strong id="skill-count">"0"</strong></span>
                <span>"MCP: "<strong id="mcp-count">"0"</strong></span>
                <span>"Invocations: "<strong id="invocation-count">"0"</strong></span>
                <span>"Events: "<strong id="event-count">"0"</strong></span>
                <span id="last-update">"-"</span>
            </div>
        </header>

        <main>
            <SkillsPanel/>
            <MetricsPanel/>
            <ActivityPanel/>
            <McpServersPanel/>
        </main>

        <footer>
            <span>"Auto-refreshes every 30s"</span>
        </footer>
    }
}

/// Skills list panel.
#[component]
pub fn SkillsPanel() -> impl IntoView {
    view! {
        <section class="panel skills-panel">
            <div class="panel-header">
                <h2>"Skills"</h2>
                <button id="sort-btn" class="sort-btn" title="Toggle sort order">"Sort: Discovery"</button>
            </div>
            <div class="skill-list" id="skill-list">
                <div class="empty">"Loading skills..."</div>
            </div>
        </section>
    }
}

/// Activity feed panel.
#[component]
pub fn ActivityPanel() -> impl IntoView {
    view! {
        <section class="panel activity-panel">
            <h2>"Activity"</h2>
            <div class="activity-list" id="activity-list">
                <div class="empty">"No recent activity"</div>
            </div>
        </section>
    }
}

/// Metrics/details panel.
#[component]
pub fn MetricsPanel() -> impl IntoView {
    view! {
        <section class="panel metrics-panel">
            <h2>"Metrics"</h2>
            <div id="metrics-content">
                <div class="empty">"Select a skill to view details"</div>
            </div>
        </section>
    }
}

/// MCP servers panel.
#[component]
pub fn McpServersPanel() -> impl IntoView {
    view! {
        <section class="panel mcp-panel">
            <h2>"MCP Servers"</h2>
            <div id="mcp-list">
                <div class="empty">"Loading MCP servers..."</div>
            </div>
        </section>
    }
}

/// The dashboard script, served at `/static/dashboard.js`.
pub(crate) const DASHBOARD_JS: &str = include_str!("dashboard.js");

/// Render the full dashboard HTML as a string for SSR.
pub fn render_dashboard() -> String {
    use leptos::tachys::view::RenderHtml;
    let view = App();
    // App component already renders <!DOCTYPE html>, so no prefix needed.
    view.to_html()
}

#[cfg(test)]
mod tests {
    use super::render_dashboard;

    /// The three desktop columns read Skills, Metrics, Activity: metrics sits
    /// next to the skills it describes and the activity feed takes the far right.
    #[test]
    fn dashboard_columns_are_skills_then_metrics_then_activity() {
        let html = render_dashboard();
        let at = |class: &str| {
            html.find(class)
                .unwrap_or_else(|| panic!("{class} missing from dashboard"))
        };
        let skills = at("panel skills-panel");
        let metrics = at("panel metrics-panel");
        let activity = at("panel activity-panel");
        assert!(
            skills < metrics && metrics < activity,
            "expected skills < metrics < activity, got {skills} {metrics} {activity}"
        );
    }
}
