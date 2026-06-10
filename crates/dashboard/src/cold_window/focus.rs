//! Pane focus model for the cold-window TUI.
//!
//! Exactly one of the three body panes (alerts, hints, research) holds
//! focus at any time. Focus governs which pane the contextual hint bar
//! describes and which pane `Enter` (drill-down) and `z` (zoom) target;
//! it does NOT gate the existing disjoint pane keymaps, which keep
//! working globally (`A`/`d`, `0`-`5`/`P`, `R`).
//!
//! `Tab` cycles forward, `Shift-Tab` (BackTab) cycles backward. The
//! focused pane's border is emphasized with a bold cyan border and a
//! `>` marker prefixed to its title so focus reads without color.

use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders};

/// Which body pane holds focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusTarget {
    /// The alert list (default focus at launch).
    #[default]
    Alerts,
    /// The ranked hint list.
    Hints,
    /// The research findings panel.
    Research,
}

impl FocusTarget {
    /// The pane after this one in `Tab` order
    /// (alerts -> hints -> research -> alerts).
    pub fn next(self) -> Self {
        match self {
            Self::Alerts => Self::Hints,
            Self::Hints => Self::Research,
            Self::Research => Self::Alerts,
        }
    }

    /// The pane before this one in `Tab` order (BackTab).
    pub fn prev(self) -> Self {
        match self {
            Self::Alerts => Self::Research,
            Self::Hints => Self::Alerts,
            Self::Research => Self::Hints,
        }
    }
}

/// Build a pane's outer `Block`, emphasizing it when focused.
///
/// Focused panes get a bold cyan border and a `>` title marker; the
/// marker keeps focus legible on monochrome terminals (semantic
/// styling, not color alone).
pub fn pane_block(title: String, focused: bool) -> Block<'static> {
    if focused {
        Block::default()
            .borders(Borders::ALL)
            .border_style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .title(format!(">{title}"))
    } else {
        Block::default().borders(Borders::ALL).title(title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::Paragraph;
    use ratatui::Terminal;

    #[test]
    fn default_focus_is_alerts() {
        assert_eq!(FocusTarget::default(), FocusTarget::Alerts);
    }

    #[test]
    fn next_cycles_alerts_hints_research() {
        assert_eq!(FocusTarget::Alerts.next(), FocusTarget::Hints);
        assert_eq!(FocusTarget::Hints.next(), FocusTarget::Research);
        assert_eq!(FocusTarget::Research.next(), FocusTarget::Alerts);
    }

    #[test]
    fn prev_is_the_inverse_of_next() {
        for f in [
            FocusTarget::Alerts,
            FocusTarget::Hints,
            FocusTarget::Research,
        ] {
            assert_eq!(f.next().prev(), f, "{f:?}: prev must invert next");
            assert_eq!(f.prev().next(), f, "{f:?}: next must invert prev");
        }
    }

    /// Render a block into a small buffer and flatten it to text.
    fn rendered_text(block: Block<'static>) -> String {
        let mut terminal = Terminal::new(TestBackend::new(30, 5)).unwrap();
        terminal
            .draw(|f| f.render_widget(Paragraph::new("x").block(block), f.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn focused_block_carries_a_marker_visible_without_color() {
        let focused = rendered_text(pane_block(" Alerts ".into(), true));
        let unfocused = rendered_text(pane_block(" Alerts ".into(), false));
        assert!(
            focused.contains("> Alerts"),
            "focused title must carry the > marker, got: {focused}"
        );
        assert!(
            !unfocused.contains('>'),
            "unfocused title must not carry the marker, got: {unfocused}"
        );
    }
}
