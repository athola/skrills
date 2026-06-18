//! Status bar widget.
//!
//! Bottom-of-screen one-liner that surfaces the cold-window's
//! operating profile so the user always knows what's active:
//!
//! - **tick rate and adaptive label**: `tick: 2.0s [base]`,
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
use ratatui::widgets::Paragraph;
use skrills_snapshot::{ResearchQuota, WindowSnapshot};

use super::focus::FocusTarget;
use super::keymap::{bindings_for, BindingScope};
use super::overlay::Overlay;
use super::state::ColdWindowState;

/// Minimum hint-segment width: room for the always-present `? help`
/// plus a truncation ellipsis (FR-2.3).
const MIN_HINT_WIDTH: u16 = 9;

/// Stateless renderer for the status bar.
pub struct StatusBar;

impl StatusBar {
    /// Render the status line into `area`: state summary on the left,
    /// the contextual key hints for `focus` right-aligned (FR-2).
    ///
    /// Rendered borderless: the bar owns a single terminal row, and a
    /// `Borders::TOP` block at height 1 used to swallow the content,
    /// leaving only a title rule on screen.
    pub fn render(
        state: &ColdWindowState,
        research_quota: Option<ResearchQuota>,
        budget_ceiling: u64,
        focus: FocusTarget,
        topmost_overlay: Option<&Overlay>,
        frame: &mut Frame<'_>,
        area: Rect,
    ) {
        let line = build_line(state, research_quota, budget_ceiling);
        frame.render_widget(Paragraph::new(line), area);

        // Hints overdraw the left segment's tail when space runs out:
        // discoverability beats a clipped token count (FR-2.3).
        let hints = hint_text(focus, topmost_overlay);
        let want = u16::try_from(hints.chars().count()).unwrap_or(u16::MAX);
        let width = want.min(area.width).max(MIN_HINT_WIDTH.min(area.width));
        let shown = truncate_with_ellipsis(&hints, width);
        let rect = Rect::new(area.right().saturating_sub(width), area.y, width, 1);
        frame.render_widget(
            Paragraph::new(Span::styled(shown, Style::default().fg(Color::DarkGray))),
            rect,
        );
    }

    /// Render to a plain string (useful for testing without a terminal
    /// backend, and for logging).
    pub fn render_to_string(
        state: &ColdWindowState,
        research_quota: Option<ResearchQuota>,
        budget_ceiling: u64,
    ) -> String {
        line_to_plain_string(&build_line(state, research_quota, budget_ceiling))
    }
}

fn build_line<'a>(
    state: &ColdWindowState,
    research_quota: Option<ResearchQuota>,
    budget_ceiling: u64,
) -> Line<'a> {
    let cadence_label = cadence_label(state.current.as_deref());
    let token_label = token_label(state.token_total(), budget_ceiling);
    let counts = state.alert_counts_by_tier();
    let alerts_label = format!(
        "W:{} C:{} A:{} S:{}",
        counts.warning, counts.caution, counts.advisory, counts.status,
    );
    // Render `available/total` so the "quota: 7/10"
    // visual contract holds even though the newtype stores `used`.
    let quota_label = research_quota
        .map(|q| format!("quota: {}/{}", q.available(), q.total()))
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

/// The contextual hint text for the current interface state (FR-2).
///
/// `? help` leads so it survives right-truncation; the focused pane's
/// keys follow, then the globals. With an overlay open, the overlay's
/// keys replace the pane keys entirely (FR-2.2); the palette gets its
/// own line because `q` types there instead of closing. Content
/// derives from the keymap table, the single source of truth.
pub fn hint_text(focus: FocusTarget, topmost_overlay: Option<&Overlay>) -> String {
    match topmost_overlay {
        Some(Overlay::Palette { .. }) => {
            return "Enter run  Up/Down select  Esc close".to_string();
        }
        Some(_) => return "? help  Esc close  q close".to_string(),
        None => {}
    }
    let scope = match focus {
        FocusTarget::Alerts => BindingScope::Alerts,
        FocusTarget::Hints => BindingScope::Hints,
        FocusTarget::Research => BindingScope::Research,
    };
    let mut parts = vec!["? help".to_string()];
    parts.push("j/k select".to_string());
    parts.push("Enter detail".to_string());
    parts.extend(
        bindings_for(scope)
            .iter()
            .map(|b| format!("{} {}", b.keys, b.action)),
    );
    parts.push("Tab panes".to_string());
    parts.push("q quit".to_string());
    parts.join("  ")
}

/// Right-truncate `s` to `width` columns, marking the cut with `…`.
fn truncate_with_ellipsis(s: &str, width: u16) -> String {
    let width = width as usize;
    if s.chars().count() <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(width - 1).collect();
    out.push('…');
    out
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

    use skrills_snapshot::{
        Alert, AlertBand, LoadSample, ResearchQuota, Severity, TokenLedger, WindowSnapshot,
    };

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
        // 3 used out of 10 → status shows available/total form (7/10).
        let s = StatusBar::render_to_string(&state, Some(ResearchQuota::new(3, 10)), 100_000);
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
            .draw(|f| {
                StatusBar::render(
                    &state,
                    None,
                    100_000,
                    FocusTarget::Alerts,
                    None,
                    f,
                    f.area(),
                )
            })
            .unwrap();
    }
}
