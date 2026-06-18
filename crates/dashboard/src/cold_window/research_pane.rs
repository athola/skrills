//! Research pane.
//!
//! Pull-only side panel that surfaces research findings asynchronously
//! attached by the tome dispatcher. Per the
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
use ratatui::widgets::{List, ListItem, Paragraph};
use skrills_snapshot::{ResearchChannel, ResearchFinding};

use super::focus::{clamped_selection, pane_block, select_row, truncate_with_ellipsis};
use super::state::ColdWindowState;

/// Mutable state owned by the research pane.
#[derive(Debug, Clone)]
pub struct ResearchPaneState {
    /// True when the pane renders as a collapsed one-liner.
    pub collapsed: bool,
    /// Fingerprint+channel combinations the user has already seen
    /// (i.e., were present in a snapshot ingested while expanded).
    pub seen_keys: HashSet<String>,
    /// Number of findings considered "new" (not yet acknowledged).
    pub badge_count: u32,
}

impl Default for ResearchPaneState {
    /// Delegates to [`ResearchPaneState::new`] so the pane starts
    /// collapsed. A `#[derive(Default)]` would set `collapsed = false`
    /// (the `bool` default) and silently open the pane on launch.
    fn default() -> Self {
        Self::new()
    }
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
    /// When the pane is **collapsed**, new findings
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
                let key = finding.to_string();
                if !self.seen_keys.contains(&key) {
                    self.badge_count = self.badge_count.saturating_add(1);
                    self.seen_keys.insert(key);
                }
            }
        } else {
            // Expanded: mark every finding as seen, badge stays 0.
            for finding in &snap.research_findings {
                self.seen_keys.insert(finding.to_string());
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
                    self.seen_keys.insert(finding.to_string());
                }
            }
            self.badge_count = 0;
        }
    }
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

/// Stateless renderer and key handler for the research pane.
pub struct ResearchPane;

impl ResearchPane {
    /// Render the pane into `area`. `focused` emphasizes the border
    /// (see [`pane_block`]); `selected` highlights one finding row in
    /// the expanded list (ignored while collapsed).
    pub fn render(
        snap_state: &ColdWindowState,
        pane_state: &ResearchPaneState,
        frame: &mut Frame<'_>,
        area: Rect,
        focused: bool,
        selected: Option<usize>,
    ) {
        if pane_state.collapsed {
            Self::render_collapsed(pane_state, frame, area, focused);
        } else {
            Self::render_expanded(snap_state, frame, area, focused, selected);
        }
    }

    fn render_collapsed(
        pane_state: &ResearchPaneState,
        frame: &mut Frame<'_>,
        area: Rect,
        focused: bool,
    ) {
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
        let paragraph = Paragraph::new(line).block(pane_block(String::new(), focused));
        frame.render_widget(paragraph, area);
    }

    fn render_expanded(
        snap_state: &ColdWindowState,
        frame: &mut Frame<'_>,
        area: Rect,
        focused: bool,
        selected: Option<usize>,
    ) {
        let snap = snap_state.current.as_deref();
        let findings: Vec<&ResearchFinding> = match snap {
            Some(s) => s.research_findings.iter().collect(),
            None => Vec::new(),
        };
        let title = format!(
            " Research  ({} findings)  press R to collapse ",
            findings.len()
        );
        let cursor = selected.and_then(|s| clamped_selection(s, findings.len()));
        let inner_width = (area.width as usize).saturating_sub(4);
        let items: Vec<ListItem> = findings
            .iter()
            .enumerate()
            .map(|(i, f)| select_row(Self::render_row(f, inner_width), cursor == Some(i)))
            .collect();
        let list = List::new(items).block(pane_block(title, focused));
        frame.render_widget(list, area);
    }

    /// Build one ratatui list row for a research finding, truncating
    /// with `"..."` when title and URL together exceed available width.
    /// Fixed chars: `" {chan:>10} "` (12) + `"  "` (2) + score (5) +
    /// `"  "` (2) = 21.
    fn render_row(finding: &ResearchFinding, inner_width: usize) -> Line<'_> {
        let channel = finding.channel.short_label();
        // " {chan:>10} " (12) + "  " (2) + score (5) + "  " (2) = 21
        let available = inner_width.saturating_sub(21);
        let title_chars = finding.title.chars().count();
        let url_chars = finding.url.chars().count();

        let mut spans = vec![
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
        ];

        if title_chars + 2 + url_chars <= available {
            spans.push(Span::raw(finding.title.clone()));
            spans.push(Span::raw(": "));
            spans.push(Span::styled(
                finding.url.clone(),
                Style::default().fg(Color::DarkGray),
            ));
        } else if title_chars + 2 < available {
            let url_space = available - title_chars - 2;
            spans.push(Span::raw(finding.title.clone()));
            spans.push(Span::raw(": "));
            spans.push(Span::styled(
                truncate_with_ellipsis(&finding.url, url_space),
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw(truncate_with_ellipsis(&finding.title, available)));
        }

        Line::from(spans)
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
    fn derived_default_matches_new_and_starts_collapsed() {
        // The live TUI builds state via `Default`, not `new()`; the two
        // must agree, otherwise research opens expanded against the
        // documented "collapsed by default" contract.
        let d = ResearchPaneState::default();
        assert!(d.collapsed, "Default must start collapsed, like new()");
        assert_eq!(d.badge_count, 0);
        assert!(d.seen_keys.is_empty());
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
    fn long_finding_row_truncates_at_narrow_width() {
        // inner_width = 40 - 4 = 36, available = 36 - 21 = 15
        // title "a very long title here" (22 chars) > 15 → must end with "..."
        let backend = TestBackend::new(40, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![ResearchFinding {
            fingerprint: "fp".into(),
            channel: ResearchChannel::GitHub,
            title: "a very long title here".into(),
            url: "https://example.com/fp".into(),
            score: 5.0,
            fetched_at_ms: 0,
        }]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.collapsed = false;
        terminal
            .draw(|f| ResearchPane::render(&snap_state, &pane_state, f, f.area(), false, None))
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
            "long title must truncate with '...', got: {text}"
        );
    }

    #[test]
    fn finding_row_truncates_url_when_title_fits_but_url_is_long() {
        // inner_width = 80 - 4 = 76, available = 76 - 21 = 55
        // title "short" (5) + ": " (2) = 7 < 55 → URL path
        // url "https://example.com/" + 50 chars > 55 - 7 = 48 → truncated
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![ResearchFinding {
            fingerprint: "fp".into(),
            channel: ResearchChannel::HackerNews,
            title: "short".into(),
            url: "https://example.com/this/is/a/very/long/url/path/that/overflows".into(),
            score: 5.0,
            fetched_at_ms: 0,
        }]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.collapsed = false;
        terminal
            .draw(|f| ResearchPane::render(&snap_state, &pane_state, f, f.area(), false, None))
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
            "title must survive when URL is truncated, got: {text}"
        );
        assert!(
            text.contains("..."),
            "truncated URL must end with '...', got: {text}"
        );
    }

    #[test]
    fn short_finding_row_needs_no_truncation() {
        let backend = TestBackend::new(120, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut snap_state = ColdWindowState::new();
        snap_state.ingest(snap(vec![finding("fp", ResearchChannel::GitHub)]));
        let mut pane_state = ResearchPaneState::new();
        pane_state.collapsed = false;
        terminal
            .draw(|f| ResearchPane::render(&snap_state, &pane_state, f, f.area(), false, None))
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
            "short finding must not be truncated, got: {text}"
        );
    }

    #[test]
    fn render_collapsed_does_not_panic() {
        let backend = TestBackend::new(80, 5);
        let mut terminal = Terminal::new(backend).unwrap();
        let snap_state = ColdWindowState::new();
        let pane_state = ResearchPaneState::new();
        terminal
            .draw(|f| ResearchPane::render(&snap_state, &pane_state, f, f.area(), false, None))
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
            .draw(|f| ResearchPane::render(&snap_state, &pane_state, f, f.area(), false, None))
            .unwrap();
    }
}
