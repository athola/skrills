//! Alert pane (TASK-015).
//!
//! Renders the visible alerts (filtered + sorted by `ColdWindowState`)
//! as a colored ratatui list. Handles two keystrokes:
//!
//! - `A` (uppercase): master-acknowledge — clears all CAUTION,
//!   ADVISORY, and STATUS alerts in one stroke per spec § 3.5.
//! - `d`: dismiss the focused WARNING-tier alert (per-row ack).
//!
//! Resize behavior (per R9 mitigation): the pane re-derives its
//! layout from the supplied `area: Rect` on every render, so a
//! crossterm `Event::Resize` simply triggers a redraw with the new
//! dimensions. No per-pane state caches the previous size.

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use skrills_snapshot::{Alert, Severity};

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

/// Stateless renderer + key handler for the alert pane.
pub struct AlertPane;

impl AlertPane {
    /// Render the visible alerts inside `area`.
    pub fn render(state: &ColdWindowState, frame: &mut Frame<'_>, area: Rect) {
        let counts = state.alert_counts_by_tier();
        let title = format!(
            " Alerts  W:{w}  C:{c}  A:{a}  S:{s} ",
            w = counts.warning,
            c = counts.caution,
            a = counts.advisory,
            s = counts.status,
        );

        let visible = state.visible_alerts();
        let items: Vec<ListItem> = visible
            .iter()
            .map(|alert| Self::render_row(alert))
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .style(Style::default());
        frame.render_widget(list, area);
    }

    /// Build one ratatui list row for an alert.
    fn render_row(alert: &Alert) -> ListItem<'_> {
        let color = tier_color(alert.severity);
        let tag = alert.severity.short_label();
        let line = Line::from(vec![
            Span::styled(
                format!(" {tag} "),
                Style::default()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
            Span::styled(alert.title.clone(), Style::default().fg(color)),
            Span::raw("  —  "),
            Span::raw(alert.message.clone()),
        ]);
        ListItem::new(line)
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
/// [`Severity::short_label`] (S1).
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
    fn render_does_not_panic_on_empty_state() {
        let backend = TestBackend::new(80, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        let state = ColdWindowState::new();
        terminal
            .draw(|f| AlertPane::render(&state, f, f.area()))
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
            .draw(|f| AlertPane::render(&state, f, f.area()))
            .unwrap();
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
                .draw(|f| AlertPane::render(&state, f, f.area()))
                .unwrap();
        }
    }
}
