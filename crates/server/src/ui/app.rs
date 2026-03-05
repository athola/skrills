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
                <script>
                    {include_str!("dashboard.js")}
                </script>
            </body>
        </html>
    }
}

/// Dashboard layout with skills panel, activity panel, and metrics panel.
#[component]
pub fn Dashboard() -> impl IntoView {
    view! {
        <header>
            <h1>"Skrills Dashboard"</h1>
            <div class="stats">
                <span>"Skills: "<strong id="skill-count">"0"</strong></span>
                <span>"Events: "<strong id="event-count">"0"</strong></span>
                <span id="last-update">"-"</span>
            </div>
        </header>

        <main>
            <SkillsPanel/>
            <ActivityPanel/>
            <MetricsPanel/>
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

/// Render the full dashboard HTML as a string for SSR.
pub fn render_dashboard() -> String {
    use leptos::tachys::view::RenderHtml;
    let view = App();
    // App component already renders <!DOCTYPE html>, so no prefix needed.
    view.to_html()
}
