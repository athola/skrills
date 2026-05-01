//! Cold-window browser surface (TASK-019 + TASK-020).
//!
//! Two endpoints:
//!
//! - `GET /dashboard` — initial HTML page with an `EventSource`
//!   pointing at `/dashboard.sse`. No JavaScript framework: the
//!   browser is a paint surface.
//! - `GET /dashboard.sse` — Server-Sent Events stream. Each tick
//!   from the bus emits four named events (`alert`, `hint`,
//!   `research`, `status`) carrying pre-rendered HTML fragments.
//!
//! HTTP/2 negotiation (per R8 mitigation): when running behind
//! TLS via `axum-server` with rustls, ALPN advertises `h2`. The
//! browser stream-multiplexes — multiple dashboard tabs in the same
//! origin all stay subscribed without bumping into HTTP/1.1's
//! 6-connection-per-origin limit.
//!
//! XSS posture: server-side fragments html-escape every user-derived
//! string. The browser then swaps fragments via `DOMParser` +
//! `replaceChildren`, which structurally prevents `<script>`
//! execution from any string that survived escaping. Two layers; if
//! either fails we degrade to "broken render", not "remote code
//! execution".

#![cfg(feature = "http-transport")]

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use async_stream::stream;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use futures::Stream;
use skrills_snapshot::{HintCategory, ResearchChannel, Severity, WindowSnapshot};
use tokio::sync::broadcast;

/// Shared state for the cold-window browser routes.
#[derive(Clone)]
pub struct ColdWindowDashboardState {
    /// Subscriber-producing handle to the engine's snapshot bus.
    pub bus: broadcast::Sender<Arc<WindowSnapshot>>,
    /// Token-budget ceiling for the status bar's progress display.
    pub budget_ceiling: u64,
    /// Optional research-quota snapshot (remaining, capacity).
    pub research_quota: Option<(u32, u32)>,
}

impl ColdWindowDashboardState {
    /// Construct from a bus + budget ceiling, no research quota.
    pub fn new(bus: broadcast::Sender<Arc<WindowSnapshot>>, budget_ceiling: u64) -> Self {
        Self {
            bus,
            budget_ceiling,
            research_quota: None,
        }
    }
}

/// Build the cold-window router.
pub fn cold_window_routes(state: ColdWindowDashboardState) -> Router {
    Router::new()
        .route("/dashboard", get(serve_dashboard))
        .route("/dashboard.sse", get(serve_dashboard_sse))
        .with_state(state)
}

async fn serve_dashboard(State(state): State<ColdWindowDashboardState>) -> Html<String> {
    Html(render_dashboard_page(state.budget_ceiling))
}

async fn serve_dashboard_sse(
    State(state): State<ColdWindowDashboardState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.bus.subscribe();
    let budget_ceiling = state.budget_ceiling;
    let research_quota = state.research_quota;

    let s = stream! {
        loop {
            match rx.recv().await {
                Ok(snap) => {
                    for event in render_snapshot_events(&snap, budget_ceiling, research_quota) {
                        yield Ok::<Event, Infallible>(event);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    let event = Event::default()
                        .event("status")
                        .data(format!(
                            "<span style=\"color:#ff5555\">subscriber lagged by {n} ticks</span>"
                        ));
                    yield Ok::<Event, Infallible>(event);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("cold-window SSE stream closing — emitting shutdown event");
                    yield Ok::<Event, Infallible>(
                        Event::default().event("shutdown").data("{}"),
                    );
                    break;
                }
            }
        }
    };

    Sse::new(s).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// Render one tick into 4 named SSE events.
fn render_snapshot_events(
    snap: &WindowSnapshot,
    budget_ceiling: u64,
    research_quota: Option<(u32, u32)>,
) -> Vec<Event> {
    vec![
        Event::default()
            .event("alert")
            .data(render_alert_fragment(snap)),
        Event::default()
            .event("hint")
            .data(render_hint_fragment(snap)),
        Event::default()
            .event("research")
            .data(render_research_fragment(snap)),
        Event::default()
            .event("status")
            .data(render_status_fragment(snap, budget_ceiling, research_quota)),
    ]
}

fn render_dashboard_page(budget_ceiling: u64) -> String {
    let budget_label = format_token_count(budget_ceiling);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<title>skrills cold-window dashboard</title>
<style>
  body {{ font-family: ui-monospace, monospace; background: #0a0a0a; color: #e0e0e0; margin: 0; padding: 16px; }}
  h1 {{ margin: 0 0 12px 0; font-size: 18px; }}
  .pane {{ border: 1px solid #444; padding: 12px; margin-bottom: 12px; border-radius: 4px; background: #121212; }}
  .pane h2 {{ margin: 0 0 8px 0; font-size: 12px; text-transform: uppercase; color: #888; }}
  .severity-warning  {{ color: #ff5555; font-weight: bold; }}
  .severity-caution  {{ color: #ffaa00; }}
  .severity-advisory {{ color: #44ddff; }}
  .severity-status   {{ color: #888; }}
  .tier-tag {{ display: inline-block; padding: 0 6px; margin-right: 8px; background: #444; color: #fff; font-size: 11px; }}
  .tier-tag.warning  {{ background: #ff5555; color: #000; }}
  .tier-tag.caution  {{ background: #ffaa00; color: #000; }}
  .tier-tag.advisory {{ background: #44ddff; color: #000; }}
  .tier-tag.status   {{ background: #888; color: #000; }}
  .pinned {{ color: #ffff00; }}
  .channel-tag {{ display: inline-block; padding: 0 6px; margin-right: 6px; font-size: 11px; }}
  .channel-tag.github   {{ background: #c000c0; color: #000; }}
  .channel-tag.hn       {{ background: #ff8800; color: #000; }}
  .channel-tag.lobsters {{ background: #cc3333; color: #000; }}
  .channel-tag.paper    {{ background: #4488ff; color: #000; }}
  .channel-tag.triz     {{ background: #00cc00; color: #000; }}
  ul {{ list-style: none; padding: 0; margin: 0; }}
  li {{ padding: 3px 0; border-bottom: 1px dashed #2a2a2a; }}
  li:last-child {{ border-bottom: 0; }}
  .empty {{ color: #555; font-style: italic; }}
  #status-bar {{ font-size: 12px; }}
  .budget-bar {{ display: inline-block; width: 200px; height: 8px; background: #222; vertical-align: middle; margin: 0 8px; border-radius: 2px; overflow: hidden; }}
  .budget-fill {{ display: block; height: 100%; background: #00cc00; }}
  .budget-fill.warn {{ background: #ffaa00; }}
  .budget-fill.crit {{ background: #ff5555; }}
</style>
</head>
<body>
<h1>skrills cold-window  ·  budget {budget_label}</h1>
<div id="status-bar" class="pane"><span class="empty">connecting…</span></div>
<section class="pane"><h2>Alerts</h2><div id="alert-body"><span class="empty">awaiting first tick…</span></div></section>
<section class="pane"><h2>Hints</h2><div id="hint-body"><span class="empty">awaiting first tick…</span></div></section>
<section class="pane"><h2>Research</h2><div id="research-body"><span class="empty">awaiting first tick…</span></div></section>
<script>
  // Defense in depth: server already html-escapes every user-derived
  // string, and the browser swaps fragments via DOMParser +
  // replaceChildren. DOMParser parses <script> tags into nodes that do
  // NOT execute when later attached to the document, so even if the
  // server-side escape ever regresses, an injected payload can't run.
  const evt = new EventSource('/dashboard.sse');
  const swap = (id, html) => {{
    const el = document.getElementById(id);
    if (!el) return;
    const parsed = new DOMParser().parseFromString(html, 'text/html');
    el.replaceChildren(...parsed.body.childNodes);
  }};
  evt.addEventListener('alert',    e => swap('alert-body',    e.data));
  evt.addEventListener('hint',     e => swap('hint-body',     e.data));
  evt.addEventListener('research', e => swap('research-body', e.data));
  evt.addEventListener('status',   e => swap('status-bar',    e.data));
  evt.onerror = () => swap('status-bar',
    '<span class="severity-warning">reconnecting…</span>');
</script>
</body>
</html>"#
    )
}

fn render_alert_fragment(snap: &WindowSnapshot) -> String {
    if snap.alerts.is_empty() {
        return r#"<span class="empty">no active alerts</span>"#.to_string();
    }
    let mut sorted = snap.alerts.iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| {
        severity_rank(a.severity)
            .cmp(&severity_rank(b.severity))
            .then(b.fired_at_ms.cmp(&a.fired_at_ms))
    });
    let mut out = String::from("<ul>");
    for alert in sorted {
        let (class, label) = severity_class(alert.severity);
        out.push_str(&format!(
            r#"<li><span class="tier-tag {class}">{label}</span><span class="severity-{class}">{title}</span> — {message}</li>"#,
            class = class,
            label = label,
            title = html_escape(&alert.title),
            message = html_escape(&alert.message),
        ));
    }
    out.push_str("</ul>");
    out
}

fn render_hint_fragment(snap: &WindowSnapshot) -> String {
    if snap.hints.is_empty() {
        return r#"<span class="empty">no hints</span>"#.to_string();
    }
    let mut sorted: Vec<_> = snap.hints.iter().collect();
    sorted.sort_by(|a, b| {
        b.pinned.cmp(&a.pinned).then_with(|| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });
    let mut out = String::from("<ul>");
    for h in sorted {
        let pin = if h.pinned { "[*] " } else { "[ ] " };
        let pin_class = if h.pinned { "pinned" } else { "" };
        out.push_str(&format!(
            r#"<li><span class="{pin_class}">{pin}</span><strong>{score:.1}</strong> [{cat}] {uri} — {msg}</li>"#,
            pin_class = pin_class,
            pin = pin,
            score = h.score,
            cat = category_label(h.hint.category),
            uri = html_escape(&h.hint.uri),
            msg = html_escape(&h.hint.message),
        ));
    }
    out.push_str("</ul>");
    out
}

fn render_research_fragment(snap: &WindowSnapshot) -> String {
    if snap.research_findings.is_empty() {
        return r#"<span class="empty">no research findings yet</span>"#.to_string();
    }
    let mut out = String::from("<ul>");
    for f in &snap.research_findings {
        let (chan_class, chan_label) = channel_classes(f.channel);
        out.push_str(&format!(
            r#"<li><span class="channel-tag {chan}">{label}</span><strong>{score:.1}</strong>  <a href="{url}" target="_blank" rel="noopener noreferrer">{title}</a></li>"#,
            chan = chan_class,
            label = chan_label,
            score = f.score,
            url = html_escape(&f.url),
            title = html_escape(&f.title),
        ));
    }
    out.push_str("</ul>");
    out
}

fn render_status_fragment(
    snap: &WindowSnapshot,
    budget_ceiling: u64,
    research_quota: Option<(u32, u32)>,
) -> String {
    let cadence = cadence_label(snap);
    let token_label = format!(
        "{} / {}",
        format_token_count(snap.token_ledger.total),
        format_token_count(budget_ceiling)
    );
    let ratio = if budget_ceiling == 0 {
        0.0
    } else {
        (snap.token_ledger.total as f64) / (budget_ceiling as f64)
    };
    let bar_class = if ratio >= 1.0 {
        "crit"
    } else if ratio >= 0.8 {
        "warn"
    } else {
        ""
    };
    let bar_width = (ratio.clamp(0.0, 1.0) * 100.0).round() as u32;
    let mut counts = [0u32; 4];
    for a in &snap.alerts {
        match a.severity {
            Severity::Warning => counts[0] += 1,
            Severity::Caution => counts[1] += 1,
            Severity::Advisory => counts[2] += 1,
            Severity::Status => counts[3] += 1,
        }
    }
    let alerts_label = format!(
        "W:{} C:{} A:{} S:{}",
        counts[0], counts[1], counts[2], counts[3]
    );
    let quota_label = research_quota
        .map(|(rem, cap)| format!("  ·  quota: {rem}/{cap}"))
        .unwrap_or_default();
    format!(
        r#"<strong>{cadence}</strong>  ·  <span>{token_label}</span><span class="budget-bar"><span class="budget-fill {bar_class}" style="width:{bar_width}%"></span></span>  ·  <span>{alerts_label}</span>{quota_label}"#
    )
}

fn cadence_label(snap: &WindowSnapshot) -> String {
    snap.cadence_label()
}

fn format_token_count(n: u64) -> String {
    if n >= 1_000 {
        format!("{:.1}K", (n as f64) / 1_000.0)
    } else {
        n.to_string()
    }
}

fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Warning => 0,
        Severity::Caution => 1,
        Severity::Advisory => 2,
        Severity::Status => 3,
    }
}

fn severity_class(severity: Severity) -> (&'static str, &'static str) {
    match severity {
        Severity::Warning => ("warning", "WARN"),
        Severity::Caution => ("caution", "CAUT"),
        Severity::Advisory => ("advisory", "ADVI"),
        Severity::Status => ("status", "STAT"),
    }
}

fn category_label(category: HintCategory) -> &'static str {
    match category {
        HintCategory::Token => "token",
        HintCategory::Validation => "validation",
        HintCategory::Redundancy => "redundancy",
        HintCategory::SyncDrift => "sync-drift",
        HintCategory::Quality => "quality",
    }
}

fn channel_classes(channel: ResearchChannel) -> (&'static str, &'static str) {
    match channel {
        ResearchChannel::GitHub => ("github", "GitHub"),
        ResearchChannel::HackerNews => ("hn", "HN"),
        ResearchChannel::Lobsters => ("lobsters", "Lobsters"),
        ResearchChannel::Paper => ("paper", "Paper"),
        ResearchChannel::Triz => ("triz", "TRIZ"),
    }
}

/// Minimal HTML escaper for user-provided text. Covers the OWASP
/// HTML attribute / body context characters; sufficient for our
/// fragments which never embed user text inside script tags or
/// onevent handlers. The browser-side `DOMParser` swap is the second
/// layer of defense (see module docs).
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use skrills_snapshot::{
        Alert, AlertBand, Hint, LoadSample, ResearchFinding, ScoredHint, TokenLedger,
    };

    fn empty_snap() -> WindowSnapshot {
        WindowSnapshot {
            version: 1,
            timestamp_ms: 0,
            token_ledger: TokenLedger::default(),
            alerts: vec![],
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        }
    }

    #[test]
    fn dashboard_page_includes_event_source_script() {
        let html = render_dashboard_page(100_000);
        assert!(html.contains("EventSource"));
        assert!(html.contains("/dashboard.sse"));
        assert!(html.contains("alert-body"));
        assert!(html.contains("hint-body"));
        assert!(html.contains("research-body"));
        assert!(html.contains("status-bar"));
    }

    #[test]
    fn dashboard_page_uses_dom_parser_replace_children() {
        let html = render_dashboard_page(100_000);
        assert!(html.contains("DOMParser"));
        assert!(html.contains("replaceChildren"));
    }

    #[test]
    fn dashboard_page_includes_budget_label() {
        let html = render_dashboard_page(100_000);
        assert!(html.contains("100.0K"));
    }

    #[test]
    fn empty_alert_fragment_says_no_active_alerts() {
        let frag = render_alert_fragment(&empty_snap());
        assert!(frag.contains("no active alerts"));
    }

    #[test]
    fn alert_fragment_sorts_warning_first() {
        let mut snap = empty_snap();
        snap.alerts = vec![
            Alert {
                fingerprint: "a1".into(),
                severity: Severity::Advisory,
                title: "advisory".into(),
                message: "m".into(),
                band: Some(AlertBand::new(0.0, 0.0, 1.0, 0.95).expect("test fixture")),
                fired_at_ms: 50,
                dwell_ticks: 1,
            },
            Alert {
                fingerprint: "w1".into(),
                severity: Severity::Warning,
                title: "warning".into(),
                message: "m".into(),
                band: None,
                fired_at_ms: 100,
                dwell_ticks: 1,
            },
        ];
        let frag = render_alert_fragment(&snap);
        let warn_idx = frag.find("warning").expect("warning label");
        let advisory_idx = frag.find("advisory").expect("advisory label");
        assert!(warn_idx < advisory_idx);
    }

    #[test]
    fn alert_fragment_escapes_user_text() {
        let mut snap = empty_snap();
        snap.alerts.push(Alert {
            fingerprint: "x".into(),
            severity: Severity::Status,
            title: "<script>evil</script>".into(),
            message: "&\"".into(),
            band: None,
            fired_at_ms: 0,
            dwell_ticks: 1,
        });
        let frag = render_alert_fragment(&snap);
        assert!(!frag.contains("<script>evil"));
        assert!(frag.contains("&lt;script&gt;"));
        assert!(frag.contains("&amp;"));
        assert!(frag.contains("&quot;"));
    }

    #[test]
    fn hint_fragment_pinned_floats_to_top() {
        let mut snap = empty_snap();
        snap.hints = vec![
            ScoredHint {
                hint: Hint {
                    uri: "low".into(),
                    category: HintCategory::Token,
                    message: "m".into(),
                    frequency: 1,
                    impact: 1.0,
                    ease_score: 1.0,
                    age_days: 0.0,
                },
                score: 0.1,
                pinned: true,
            },
            ScoredHint {
                hint: Hint {
                    uri: "high".into(),
                    category: HintCategory::Token,
                    message: "m".into(),
                    frequency: 1,
                    impact: 1.0,
                    ease_score: 1.0,
                    age_days: 0.0,
                },
                score: 99.0,
                pinned: false,
            },
        ];
        let frag = render_hint_fragment(&snap);
        let low_idx = frag.find("low").expect("low uri");
        let high_idx = frag.find("high").expect("high uri");
        assert!(low_idx < high_idx, "pinned hint must come first");
    }

    #[test]
    fn research_fragment_renders_url_and_title() {
        let mut snap = empty_snap();
        snap.research_findings.push(ResearchFinding {
            fingerprint: "fp".into(),
            channel: ResearchChannel::HackerNews,
            title: "Test Finding".into(),
            url: "https://example.com/x".into(),
            score: 142.0,
            fetched_at_ms: 0,
        });
        let frag = render_research_fragment(&snap);
        assert!(frag.contains("Test Finding"));
        assert!(frag.contains("https://example.com/x"));
        assert!(frag.contains("rel=\"noopener noreferrer\""));
    }

    #[test]
    fn research_fragment_handles_empty_findings() {
        let frag = render_research_fragment(&empty_snap());
        assert!(frag.contains("no research findings yet"));
    }

    #[test]
    fn status_fragment_includes_cadence_token_alert_quota() {
        let mut snap = empty_snap();
        snap.token_ledger.total = 25_000;
        snap.next_tick_ms = 4_000;
        snap.load_sample.loadavg_1min = 0.78;
        let frag = render_status_fragment(&snap, 100_000, Some((7, 10)));
        assert!(frag.contains("tick: 4.0s"));
        assert!(frag.contains("[load 0.78]"));
        assert!(frag.contains("25.0K / 100.0K"));
        assert!(frag.contains("W:0 C:0 A:0 S:0"));
        assert!(frag.contains("quota: 7/10"));
    }

    #[test]
    fn status_fragment_omits_quota_when_unset() {
        let snap = empty_snap();
        let frag = render_status_fragment(&snap, 100_000, None);
        assert!(!frag.contains("quota:"));
    }

    #[test]
    fn status_fragment_uses_active_edit_label() {
        let mut snap = empty_snap();
        snap.load_sample.last_edit_age_ms = Some(3_000);
        let frag = render_status_fragment(&snap, 100_000, None);
        assert!(frag.contains("[active edit]"));
    }

    #[test]
    fn status_fragment_budget_bar_has_warn_class_above_eighty_percent() {
        let mut snap = empty_snap();
        snap.token_ledger.total = 85_000;
        let frag = render_status_fragment(&snap, 100_000, None);
        assert!(frag.contains("budget-fill warn"));
    }

    #[test]
    fn status_fragment_budget_bar_has_crit_class_at_or_above_one_hundred_percent() {
        let mut snap = empty_snap();
        snap.token_ledger.total = 110_000;
        let frag = render_status_fragment(&snap, 100_000, None);
        assert!(frag.contains("budget-fill crit"));
    }

    #[test]
    fn render_snapshot_events_emits_four_named_events() {
        let snap = empty_snap();
        let events = render_snapshot_events(&snap, 100_000, None);
        assert_eq!(events.len(), 4);
    }

    #[test]
    fn html_escape_handles_basic_xss_chars() {
        assert_eq!(html_escape("<>&\"'"), "&lt;&gt;&amp;&quot;&#x27;");
        assert_eq!(html_escape("plain"), "plain");
    }

    #[tokio::test]
    async fn sse_route_responds_with_text_event_stream() {
        use axum::body::Body;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let (tx, _rx) = broadcast::channel(16);
        let state = ColdWindowDashboardState::new(tx.clone(), 100_000);
        let app = cold_window_routes(state);

        let _ = tx.send(Arc::new(empty_snap()));

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/dashboard.sse")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let content_type = response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(content_type.starts_with("text/event-stream"));
        let mut body = response.into_body();
        let chunk = body.frame().await;
        assert!(chunk.is_some(), "SSE stream produced no frames");
    }

    #[tokio::test]
    async fn sse_emits_shutdown_event_when_bus_closes() {
        // NI13/N8: previously the `RecvError::Closed` arm broke
        // without telling the client the stream was ending. Now we
        // emit a final `event: shutdown` so connected dashboards can
        // distinguish "server going down cleanly" from "network
        // hiccup, please reconnect".
        use axum::body::Body;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let (tx, _rx) = broadcast::channel::<Arc<WindowSnapshot>>(16);
        let state = ColdWindowDashboardState::new(tx.clone(), 100_000);
        let app = cold_window_routes(state);

        // Drop the only sender so the receiver's next `recv` returns
        // `RecvError::Closed`. We do this BEFORE issuing the request
        // so the handler subscribes to an already-closing channel.
        drop(tx);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/dashboard.sse")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);

        // Drain the body. Because the channel is closed and we never
        // sent a snapshot, the only event emitted should be the
        // shutdown sentinel followed by stream end.
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8_lossy(&bytes).to_string();
        assert!(
            body.contains("event:shutdown") || body.contains("event: shutdown"),
            "missing shutdown event in SSE body:\n{body}"
        );
    }

    #[tokio::test]
    async fn dashboard_html_route_returns_200_with_event_source_script() {
        use axum::body::Body;
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let (tx, _rx) = broadcast::channel(16);
        let state = ColdWindowDashboardState::new(tx, 100_000);
        let app = cold_window_routes(state);

        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/dashboard")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("EventSource"));
        assert!(body.contains("/dashboard.sse"));
        assert!(body.contains("DOMParser"));
    }
}
