//! Research pane (TASK-017).
//!
//! Pull-only side panel that surfaces research findings asynchronously
//! attached by the tome dispatcher (T011). Per spec § 3.7 and the
//! CHI 2025 contrarian finding (Theme 2 of the discourse research),
//! this pane is **collapsed by default**. Users open it when curious;
//! findings never auto-expand into the user's view.
//!
//! State:
//!
//! - `collapsed`: when true, the pane renders as a one-liner
//!   "Research [N new]" with a badge counter and no list.
//! - `seen_fingerprints`: tracks which finding fingerprints have
//!   already been counted so the badge only increments on actually-new
//!   findings.
//!
//! Keystroke: `R` toggles `collapsed`. Opening the pane resets the
//! badge counter to 0 (the user has acknowledged what's new).

use std::collections::HashSet;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use skrills_snapshot::{ResearchChannel, ResearchFinding};

use super::state::ColdWindowState;

/// Mutable state owned by the research pane.
#[derive(Debug, Clone, Default)]
pub struct ResearchPaneState {
    /// True when the pane renders as a collapsed one-liner.
    pub collapsed: bool,
    /// Fingerprint+channel combinations the user has already seen
    /// (i.e., were present in a snapshot ingested while expanded).
    pub seen_keys: HashSet<String>,
    /// Number of findings considered "new" (not yet acknowledged).
    pub badge_count: u32,
}

impl ResearchPaneState {
    /// Construct a default state (collapsed, no findings seen).
    pub fn new() -> Self {
        Self {
            collapsed: true,
            seen_keys: HashSet::new(),
            badge_count: 0,
        }
    }

    /// Re-evaluate the badge count given a new snapshot. Findings
    /// not yet in `seen_keys` count as new.
    ///
    /// Per spec § 3.7: when the pane is **collapsed**, new findings
    /// bump `badge_count`. When the pane is **expanded**, the user
    /// is actively viewing them; they're acknowledged on next ingest.
    pub fn ingest(&mut self, snap_state: &ColdWindowState) {
        let snap = match snap_state.current.as_deref() {
            Some(s) => s,
            None => return,
        };
        if self.collapsed {
            // Count only previously-unseen findings.
            for finding in &snap.research_findings {
                let key = key_for(finding);
                if !self.seen_keys.contains(&key) {
                    self.badge_count = self.badge_count.saturating_add(1);
                    self.seen_keys.insert(key);
                }
            }
        } else {
            // Expanded: mark every finding as seen, badge stays 0.
            for finding in &snap.research_findings {
                self.seen_keys.insert(key_for(finding));
            }
            self.badge_count = 0;
        }
    }

    /// Toggle the collapsed/expanded state. When opening, mark
    /// everything as seen (badge clears).
    pub fn toggle(&mut self, snap_state: &ColdWindowState) {
        self.collapsed = !self.collapsed;
        if !self.collapsed {
            // Opening: ack everything visible.
            if let Some(snap) = snap_state.current.as_deref() {
                for finding in &snap.research_findings {
                    self.seen_keys.insert(key_for(finding));
                }
            }
            self.badge_count = 0;
        }
    }
}

fn key_for(finding: &ResearchFinding) -> String {
    format!("{}#{:?}", finding.fingerprint, finding.channel)
}

/// Action returned by the pane after handling a keystroke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResearchAction {
    /// No state change.
    NoOp,
    /// Pane toggled.
    Toggled {
        /// New collapsed state (true = collapsed after toggle).
        collapsed: bool,
    },
}

/// Stateless renderer + key handler for the research pane.
pub struct ResearchPane;

impl ResearchPane {
    /// Render the pane into `area`.
    pub fn render(
        snap_state: &ColdWindowState,
        pane_state: &ResearchPaneState,
        frame: &mut Frame<'_>,
        area: Rect,
    ) {
        if pane_state.collapsed {
            Self::render_collapsed(pane_state, frame, area);
        } else {
            Self::render_expanded(snap_state, frame, area);
        }
    }

    fn render_collapsed(pane_state: &ResearchPaneState, frame: &mut Frame<'_>, area: Rect) {
        let badge = if pane_state.badge_count > 0 {
            Span::styled(
                format!(" [{} new] ", pane_state.badge_count),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(" [no new] ", Style::default().fg(Color::DarkGray))
        };
        let line = Line::from(vec![
            Span::styled(
                " Research ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            badge,
            Span::raw("  press R to expand"),
        ]);
        let paragraph = Paragraph::new(line).block(Block::default().borders(Borders::ALL));
        frame.render_widget(paragraph, area);
    }

    fn render_expanded(snap_state: &ColdWindowState, frame: &mut Frame<'_>, area: Rect) {
        let snap = snap_state.current.as_deref();
        let findings: Vec<&ResearchFinding> = match snap {
            Some(s) => s.research_findings.iter().collect(),
            None => Vec::new(),
        };
        let title = format!(
            " Research  ({} findings)  press R to collapse ",
            findings.len()
        );
        let items: Vec<ListItem> = findings.iter().map(|f| Self::render_row(f)).collect();
        let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
        frame.render_widget(list, area);
    }

    fn render_row(finding: &ResearchFinding) -> ListItem<'_> {
        let channel = channel_label(finding.channel);
        let line = Line::from(vec![
            Span::styled(
                format!(" {channel:>10} "),
                Style::default()
                    .fg(Color::Black)
                    .bg(channel_color(finding.channel))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                format!("{:>5.1}", finding.score),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::raw(finding.title.clone()),
            Span::raw("  —  "),
            Span::styled(finding.url.clone(), Style::default().fg(Color::DarkGray)),
        ]);
        ListItem::new(line)
    }

    /// Handle a keystroke. `R` toggles collapsed/expanded.
    pub fn handle_key(
        snap_state: &ColdWindowState,
        pane_state: &mut ResearchPaneState,
        key: KeyCode,
    ) -> ResearchAction {
        match key {
            KeyCode::Char('R') => {
                pane_state.toggle(snap_state);
                ResearchAction::Toggled {
                    collapsed: pane_state.collapsed,
                }
            }
            _ => ResearchAction::NoOp,
        }
    }
}

fn channel_label(channel: ResearchChannel) -> &'static str {
    match channel {
        ResearchChannel::GitHub => "GitHub",
        ResearchChannel::HackerNews => "HN",
        ResearchChannel::Lobsters => "Lobsters",
        ResearchChannel::Paper => "Paper",
        ResearchChannel::Triz => "TRIZ",
    }
}

fn channel_color(channel: ResearchChannel) -> Color {
    match channel {
        ResearchChannel::GitHub => Color::Magenta,
        ResearchChannel::HackerNews => Color::Yellow,
        ResearchChannel::Lobsters => Color::Red,
        ResearchChannel::Paper => Color::Blue,
        ResearchChannel::Triz => Color::Green,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use ratatui::backend::TestBackend;
    use skrills_snapshot::{LoadSample, TokenLedger, WindowSnapshot};

    fn finding(fp: &str, channel: ResearchChannel) -> ResearchFinding {
        ResearchFinding {
            fingerprint: fp.to_string(),
            channel,
            title: format!("title-{fp}"),
            url: format!("https://example.com/{fp}"),
            score: 5.0,
            fetched_at_ms: 0,
        }
    }

    fn snap(findings: Vec<ResearchFinding>) -> Arc<WindowSnapshot> {
        Arc::new(WindowSnapshot {
            version: 1,
            timestamp_ms: 0,
            token_ledger: TokenLedger::default(),
            alerts: vec![],
            hints: vec![],
            research_findings: findings,
            plugin_health: vec![],
            load_sample: LoadSample::default(),
            next_tick_ms: 2_000,
        })
    }

    #[test]
    fn default_state_is_collapsed_with_no_badge() {
        let s = ResearchPaneState::new();
        assert!(s.collapsed);
        assert_eq!(s.badge_count, 0);
    }

    #[test]
    fn ingest_when_collapsed_increments_badge_for_new_findings() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![
            finding("fp1", ResearchChannel::GitHub),
            finding("fp2", ResearchChannel::HackerNews),
        ]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.ingest(&snap_state);
        assert_eq!(pane_state.badge_count, 2);
    }

    #[test]
    fn ingest_does_not_double_count_seen_findings() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![finding("fp1", ResearchChannel::GitHub)]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.ingest(&snap_state);
        pane_state.ingest(&snap_state);
        assert_eq!(pane_state.badge_count, 1);
    }

    #[test]
    fn ingest_when_expanded_keeps_badge_at_zero() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![finding("fp1", ResearchChannel::GitHub)]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.collapsed = false;
        pane_state.ingest(&snap_state);
        assert_eq!(pane_state.badge_count, 0);
    }

    #[test]
    fn toggle_to_expanded_clears_badge() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![finding("fp1", ResearchChannel::GitHub)]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.ingest(&snap_state);
        assert_eq!(pane_state.badge_count, 1);
        pane_state.toggle(&snap_state);
        assert!(!pane_state.collapsed);
        assert_eq!(pane_state.badge_count, 0);
    }

    #[test]
    fn toggle_keystroke_returns_action() {
        let snap_state = ColdWindowState::new();
        let mut pane_state = ResearchPaneState::new();
        let action = ResearchPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('R'));
        assert_eq!(action, ResearchAction::Toggled { collapsed: false });
    }

    #[test]
    fn unrelated_keystroke_is_noop() {
        let snap_state = ColdWindowState::new();
        let mut pane_state = ResearchPaneState::new();
        let action = ResearchPane::handle_key(&snap_state, &mut pane_state, KeyCode::Char('q'));
        assert_eq!(action, ResearchAction::NoOp);
    }

    #[test]
    fn channel_distinguishes_findings_with_same_fingerprint() {
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![
            finding("same", ResearchChannel::GitHub),
            finding("same", ResearchChannel::HackerNews),
        ]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.ingest(&snap_state);
        assert_eq!(
            pane_state.badge_count, 2,
            "different channels with same fingerprint should both count"
        );
    }

    #[test]
    fn render_collapsed_does_not_panic() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let snap_state = ColdWindowState::new();
        let pane_state = ResearchPaneState::new();
        terminal
            .draw(|f| ResearchPane::render(&snap_state, &pane_state, f, f.area()))
            .unwrap();
    }

    #[test]
    fn render_expanded_with_findings_does_not_panic() {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![
            finding("fp1", ResearchChannel::GitHub),
            finding("fp2", ResearchChannel::Triz),
        ]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.collapsed = false;
        terminal
            .draw(|f| ResearchPane::render(&snap_state, &pane_state, f, f.area()))
            .unwrap();
    }
}
