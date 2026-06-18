//! Alert pane.
//!
//! Renders the visible alerts (filtered and sorted by `ColdWindowState`)
//! as a colored ratatui list. Handles two keystrokes:
//!
//! - `A` (uppercase): master-acknowledge, clears all CAUTION,
//!   ADVISORY, and STATUS alerts in one stroke.
//! - `d`: dismiss the focused WARNING-tier alert (per-row ack).
//!
//! Resize behavior: the pane re-derives its
//! layout from the supplied `area: Rect` on every render, so a
//! crossterm `Event::Resize` simply triggers a redraw with the new
//! dimensions. No per-pane state caches the previous size.

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{List, ListItem};
use skrills_snapshot::{Alert, Severity};

use super::focus::{clamped_selection, pane_block, select_row, truncate_with_ellipsis};
use super::state::ColdWindowState;

/// Action returned by the pane after handling a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertAction {
    /// No state change.
    NoOp,
    /// User pressed master-ack; `cleared` non-warning alerts dismissed.
    MasterAcked {
        /// Number of CAUTION/ADVISORY/STATUS alerts dismissed.
        cleared: usize,
    },
    /// User dismissed a single WARNING-tier alert by fingerprint.
    WarningAcked {
        /// Fingerprint of the dismissed warning.
        fingerprint: String,
    },
}

/// Stateless renderer and key handler for the alert pane.
pub struct AlertPane;

impl AlertPane {
    /// Render the visible alerts inside `area`. `focused` emphasizes
    /// the border (see [`pane_block`]); `selected` highlights one row
    /// with the `> ` cursor (pass `None` when the pane is unfocused).
    pub fn render(
        state: &ColdWindowState,
        frame: &mut Frame<'_>,
        area: Rect,
        focused: bool,
        selected: Option<usize>,
    ) {
        let counts = state.alert_counts_by_tier();
        let title = format!(
            " Alerts  W:{w}  C:{c}  A:{a}  S:{s} ",
            w = counts.warning,
            c = counts.caution,
            a = counts.advisory,
            s = counts.status,
        );

        let visible = state.visible_alerts();
        let cursor = selected.and_then(|s| clamped_selection(s, visible.len()));
        let inner_width = (area.width as usize).saturating_sub(4);
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(i, alert)| select_row(Self::render_row(alert, inner_width), cursor == Some(i)))
            .collect();

        let list = List::new(items)
            .block(pane_block(title, focused))
            .style(Style::default());
        frame.render_widget(list, area);
    }

    /// Build one ratatui list row for an alert, truncating with `"..."`
    /// when the title and message together exceed `inner_width - 7`
    /// characters. The 7 fixed chars are the severity badge `" WARN "`
    /// (6) plus the space that follows (1).
    fn render_row(alert: &Alert, inner_width: usize) -> Line<'_> {
        let color = tier_color(alert.severity);
        let tag = alert.severity.short_label();
        // " WARN " (6) + " " (1)
        let available = inner_width.saturating_sub(7);
        let title_chars = alert.title.chars().count();
        let msg_chars = alert.message.chars().count();

        let mut spans = vec![
            Span::styled(
                format!(" {tag} "),
                Style::default()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ];

        if title_chars + 2 + msg_chars <= available {
            spans.push(Span::styled(
                alert.title.clone(),
                Style::default().fg(color),
            ));
            spans.push(Span::raw(": "));
            spans.push(Span::raw(alert.message.clone()));
        } else if title_chars + 2 < available {
            let msg_space = available - title_chars - 2;
            spans.push(Span::styled(
                alert.title.clone(),
                Style::default().fg(color),
            ));
            spans.push(Span::raw(": "));
            spans.push(Span::raw(truncate_with_ellipsis(&alert.message, msg_space)));
        } else {
            spans.push(Span::styled(
                truncate_with_ellipsis(&alert.title, available),
                Style::default().fg(color),
            ));
        }

        Line::from(spans)
    }

    /// Handle a keystroke; mutate state if relevant and return the action.
    pub fn handle_key(state: &mut ColdWindowState, key: KeyCode) -> AlertAction {
        match key {
            KeyCode::Char('A') => {
                let cleared = state.master_ack();
                AlertAction::MasterAcked { cleared }
            }
            KeyCode::Char('d') => {
                // Dismiss the highest-priority visible warning.
                if let Some(fingerprint) = state.visible_alerts().iter().find_map(|a| {
                    if matches!(a.severity, Severity::Warning) {
                        Some(a.fingerprint.clone())
                    } else {
                        None
                    }
                }) {
                    state.ack_warning(&fingerprint);
                    AlertAction::WarningAcked { fingerprint }
                } else {
                    AlertAction::NoOp
                }
            }
            _ => AlertAction::NoOp,
        }
    }
}

/// Map a severity tier to its ratatui color. Labels live on
/// [`Severity::short_label`].
fn tier_color(severity: Severity) -> Color {
    match severity {
        Severity::Warning => Color::Red,
        Severity::Caution => Color::Yellow,
        Severity::Advisory => Color::Cyan,
        Severity::Status => Color::Gray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use std::sync::Arc;

    use skrills_snapshot::{AlertBand, LoadSample, TokenLedger, WindowSnapshot};

    fn alert(fingerprint: &str, severity: Severity) -> Alert {
        Alert {
            fingerprint: fingerprint.to_string(),
            severity,
            title: format!("title-{fingerprint}"),
            message: format!("message-{fingerprint}"),
            band: Some(AlertBand::new(0.0, 0.0, 1.0, 0.95).expect("test fixture")),
            fired_at_ms: 100,
            dwell_ticks: 1,
        }
    }

    fn snap(alerts: Vec<Alert>) -> Arc<WindowSnapshot> {
        Arc::new(WindowSnapshot {
            version: 1,
            timestamp_ms: 0,
            token_ledger: TokenLedger::default(),
            alerts,
            hints: vec![],
            research_findings: vec![],
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        })
    }

    #[test]
    fn master_ack_keystroke_returns_cleared_count() {
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![
            alert("c1", Severity::Caution),
            alert("a1", Severity::Advisory),
            alert("w1", Severity::Warning),
        ]));
        let action = AlertPane::handle_key(&mut state, KeyCode::Char('A'));
        assert_eq!(action, AlertAction::MasterAcked { cleared: 2 });
    }

    #[test]
    fn dismiss_keystroke_acks_first_warning() {
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![
            alert("w1", Severity::Warning),
            alert("w2", Severity::Warning),
        ]));
        let action = AlertPane::handle_key(&mut state, KeyCode::Char('d'));
        match action {
            AlertAction::WarningAcked { fingerprint } => {
                assert!(state.acked_warnings.contains(&fingerprint));
            }
            other => panic!("expected WarningAcked, got {other:?}"),
        }
    }

    #[test]
    fn dismiss_with_no_warnings_is_noop() {
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![alert("c1", Severity::Caution)]));
        let action = AlertPane::handle_key(&mut state, KeyCode::Char('d'));
        assert_eq!(action, AlertAction::NoOp);
    }

    #[test]
    fn unrelated_keystroke_is_noop() {
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![alert("w1", Severity::Warning)]));
        let action = AlertPane::handle_key(&mut state, KeyCode::Char('q'));
        assert_eq!(action, AlertAction::NoOp);
    }

    #[test]
    fn severity_stays_textual_so_meaning_survives_without_color() {
        // FR-7.2: tier colors are decoration; the severity tag itself
        // is text, so a monochrome terminal loses nothing semantic.
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![
            alert("w1", Severity::Warning),
            alert("c1", Severity::Caution),
        ]));
        terminal
            .draw(|f| AlertPane::render(&state, f, f.area(), false, None))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        for severity in [Severity::Warning, Severity::Caution] {
            assert!(
                text.contains(severity.short_label()),
                "severity {severity:?} must appear as text, got: {text}"
            );
        }
    }

    #[test]
    fn render_does_not_panic_on_empty_state() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = ColdWindowState::new();
        terminal
            .draw(|f| AlertPane::render(&state, f, f.area(), false, None))
            .unwrap();
    }

    #[test]
    fn render_handles_many_alerts() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = ColdWindowState::new();
        let alerts: Vec<Alert> = (0..50)
            .map(|i| alert(&format!("a{i}"), Severity::Status))
            .collect();
        state.ingest(snap(alerts));
        terminal
            .draw(|f| AlertPane::render(&state, f, f.area(), false, None))
            .unwrap();
    }

    #[test]
    fn long_alert_row_truncates_at_narrow_width() {
        // inner_width = 30 - 4 = 26, available = 26 - 7 = 19
        // title (25 chars) > available (19) → row must end with "..."
        let backend = TestBackend::new(30, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![Alert {
            fingerprint: "x".into(),
            severity: Severity::Warning,
            title: "this title is very long indeed".into(),
            message: "m".into(),
            band: Some(AlertBand::new(0.0, 0.0, 1.0, 0.95).expect("test")),
            fired_at_ms: 0,
            dwell_ticks: 1,
        }]));
        terminal
            .draw(|f| AlertPane::render(&state, f, f.area(), false, None))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            text.contains("..."),
            "long row must end with '...', got: {text}"
        );
    }

    #[test]
    fn alert_row_truncates_message_when_title_fits_but_message_is_long() {
        // inner_width = 60 - 4 = 56, available = 56 - 7 = 49
        // title "short" (5) + ": " (2) = 7 < 49 → message path
        // message 50 chars > 49 - 7 = 42 → truncated with "..."
        let backend = TestBackend::new(60, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![Alert {
            fingerprint: "z".into(),
            severity: Severity::Advisory,
            title: "short".into(),
            message: "this message is long enough to overflow the available space indeed".into(),
            band: None,
            fired_at_ms: 0,
            dwell_ticks: 1,
        }]));
        terminal
            .draw(|f| AlertPane::render(&state, f, f.area(), false, None))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            text.contains("short"),
            "title must survive when message is truncated, got: {text}"
        );
        assert!(
            text.contains("..."),
            "truncated message must end with '...', got: {text}"
        );
    }

    #[test]
    fn short_alert_row_needs_no_truncation() {
        // title "AB" + ": " + message "CD" = 6 chars; fits at any sane width.
        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![Alert {
            fingerprint: "y".into(),
            severity: Severity::Status,
            title: "AB".into(),
            message: "CD".into(),
            band: None,
            fired_at_ms: 0,
            dwell_ticks: 1,
        }]));
        terminal
            .draw(|f| AlertPane::render(&state, f, f.area(), false, None))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            !text.contains("..."),
            "short row must not be truncated, got: {text}"
        );
    }

    #[test]
    fn render_after_resize_does_not_panic() {
        // R9: TUI must redraw cleanly on terminal resize.
        let mut state = ColdWindowState::new();
        state.ingest(snap(vec![alert("w1", Severity::Warning)]));
        for size in [(40, 10), (120, 30), (20, 5), (200, 50)] {
            let backend = TestBackend::new(size.0, size.1);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| AlertPane::render(&state, f, f.area(), false, None))
                .unwrap();
        }
    }
}
