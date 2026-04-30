//! Status bar widget (TASK-018).
//!
//! Bottom-of-screen one-liner that surfaces the cold-window's
//! operating profile so the user always knows what's active:
//!
//! - **tick rate + adaptive label**: `tick: 2.0s [base]`,
//!   `tick: 4.0s [load 0.78]`, `tick: 1.0s [active edit]`.
//! - **token budget**: `68K / 100K`.
//! - **alert counts per tier**: `W:1 C:0 A:2 S:0`.
//! - **research-quota remaining**: `quota: 7/10`.
//!
//! Implemented as a stateless renderer that derives all display
//! values from `ColdWindowState`'s current snapshot.

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use skrills_snapshot::WindowSnapshot;

use super::state::ColdWindowState;

/// Stateless renderer for the status bar.
pub struct StatusBar;

impl StatusBar {
    /// Render the status line into `area`.
    pub fn render(
        state: &ColdWindowState,
        research_quota: Option<(u32, u32)>,
        budget_ceiling: u64,
        frame: &mut Frame<'_>,
        area: Rect,
    ) {
        let line = build_line(state, research_quota, budget_ceiling);
        let paragraph =
            Paragraph::new(line).block(Block::default().borders(Borders::TOP).title(" Status "));
        frame.render_widget(paragraph, area);
    }

    /// Render to a plain string (useful for testing without a terminal
    /// backend, and for logging).
    pub fn render_to_string(
        state: &ColdWindowState,
        research_quota: Option<(u32, u32)>,
        budget_ceiling: u64,
    ) -> String {
        line_to_plain_string(&build_line(state, research_quota, budget_ceiling))
    }
}

fn build_line<'a>(
    state: &ColdWindowState,
    research_quota: Option<(u32, u32)>,
    budget_ceiling: u64,
) -> Line<'a> {
    let cadence_label = cadence_label(state.current.as_deref());
    let token_label = token_label(state.token_total(), budget_ceiling);
    let counts = state.alert_counts_by_tier();
    let alerts_label = format!(
        "W:{} C:{} A:{} S:{}",
        counts.warning, counts.caution, counts.advisory, counts.status,
    );
    let quota_label = research_quota
        .map(|(remaining, capacity)| format!("quota: {remaining}/{capacity}"))
        .unwrap_or_default();

    let mut spans = vec![
        Span::styled(
            cadence_label,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            token_label,
            token_style(state.token_total(), budget_ceiling),
        ),
        Span::raw("  "),
        Span::styled(alerts_label, Style::default().fg(Color::Cyan)),
    ];
    if !quota_label.is_empty() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            quota_label,
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn line_to_plain_string(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect::<Vec<&str>>()
        .join("")
}

/// Format the cadence label, including the adaptive-state suffix.
///
/// The shared formatting (post-warmup) lives on `WindowSnapshot` so
/// that the HTTP SSE handler and this TUI status bar emit byte-
/// identical strings; only the warmup branch differs.
fn cadence_label(snapshot: Option<&WindowSnapshot>) -> String {
    match snapshot {
        None => "tick: -- [warmup]".to_string(),
        Some(s) => s.cadence_label(),
    }
}

fn token_label(total: u64, ceiling: u64) -> String {
    fn fmt(n: u64) -> String {
        if n >= 1_000 {
            format!("{:.1}K", (n as f64) / 1_000.0)
        } else {
            n.to_string()
        }
    }
    format!("{} / {}", fmt(total), fmt(ceiling))
}

fn token_style(total: u64, ceiling: u64) -> Style {
    let ratio = if ceiling == 0 {
        0.0
    } else {
        (total as f64) / (ceiling as f64)
    };
    let color = if ratio >= 1.0 {
        Color::Red
    } else if ratio >= 0.8 {
        Color::Yellow
    } else if ratio >= 0.5 {
        Color::Cyan
    } else {
        Color::Green
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::sync::Arc;

    use skrills_snapshot::{Alert, AlertBand, LoadSample, Severity, TokenLedger, WindowSnapshot};

    fn snap_with(
        load: f64,
        last_edit_ms: Option<u64>,
        tokens: u64,
        next_tick_ms: u64,
    ) -> Arc<WindowSnapshot> {
        Arc::new(WindowSnapshot {
            version: 1,
            timestamp_ms: 0,
            token_ledger: TokenLedger {
                total: tokens,
                ..Default::default()
            },
            alerts: vec![],
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample {
                loadavg_1min: load,
                last_edit_age_ms: last_edit_ms,
            },
            next_tick_ms,
        })
    }

    #[test]
    fn warmup_label_when_no_snapshot() {
        let state = ColdWindowState::new();
        let s = StatusBar::render_to_string(&state, None, 100_000);
        assert!(s.contains("warmup"));
    }

    #[test]
    fn active_edit_label_when_recent_edit() {
        let mut state = ColdWindowState::new();
        state.ingest(snap_with(0.5, Some(2_000), 5_000, 1_000));
        let s = StatusBar::render_to_string(&state, None, 100_000);
        assert!(s.contains("active edit"), "got: {s}");
        assert!(s.contains("tick: 1.0s"));
    }

    #[test]
    fn load_label_when_loaded_no_edit() {
        let mut state = ColdWindowState::new();
        state.ingest(snap_with(2.5, None, 5_000, 4_000));
        let s = StatusBar::render_to_string(&state, None, 100_000);
        assert!(s.contains("load 2.50"), "got: {s}");
        assert!(s.contains("tick: 4.0s"));
    }

    #[test]
    fn base_label_when_idle() {
        let mut state = ColdWindowState::new();
        state.ingest(snap_with(0.0, None, 5_000, 2_000));
        let s = StatusBar::render_to_string(&state, None, 100_000);
        assert!(s.contains("[base]"), "got: {s}");
    }

    #[test]
    fn token_label_uses_k_for_large_numbers() {
        let mut state = ColdWindowState::new();
        state.ingest(snap_with(0.0, None, 25_000, 2_000));
        let s = StatusBar::render_to_string(&state, None, 100_000);
        assert!(s.contains("25.0K / 100.0K"), "got: {s}");
    }

    #[test]
    fn quota_label_renders_when_provided() {
        let mut state = ColdWindowState::new();
        state.ingest(snap_with(0.0, None, 5_000, 2_000));
        let s = StatusBar::render_to_string(&state, Some((7, 10)), 100_000);
        assert!(s.contains("quota: 7/10"), "got: {s}");
    }

    #[test]
    fn alert_counts_appear_in_label() {
        let mut state = ColdWindowState::new();
        let alert = Alert {
            fingerprint: "w1".into(),
            severity: Severity::Warning,
            title: "t".into(),
            message: "m".into(),
            band: Some(AlertBand::new(0.0, 0.0, 1.0, 0.95).expect("test fixture")),
            fired_at_ms: 0,
            dwell_ticks: 1,
        };
        let snap = Arc::new(WindowSnapshot {
            version: 1,
            timestamp_ms: 0,
            token_ledger: TokenLedger::default(),
            alerts: vec![alert],
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        });
        state.ingest(snap);
        let s = StatusBar::render_to_string(&state, None, 100_000);
        assert!(s.contains("W:1"), "got: {s}");
    }

    #[test]
    fn render_does_not_panic_at_minimum_size() {
        // R9 again: status bar must redraw cleanly at small widths.
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = ColdWindowState::new();
        terminal
            .draw(|f| StatusBar::render(&state, None, 100_000, f, f.area()))
            .unwrap();
    }
}
