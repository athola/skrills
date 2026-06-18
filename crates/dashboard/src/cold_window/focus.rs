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
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, ListItem};

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

/// Wrap a list row in the selection treatment: a `> ` marker prefix
/// plus `REVERSED` styling when selected, a two-space gutter when not.
/// The marker keeps the cursor legible without color (FR-5/T5).
pub fn select_row(mut line: Line<'_>, selected: bool) -> ListItem<'_> {
    let marker = if selected { "> " } else { "  " };
    line.spans.insert(
        0,
        Span::styled(marker, Style::default().add_modifier(Modifier::BOLD)),
    );
    let item = ListItem::new(line);
    if selected {
        item.style(Style::default().add_modifier(Modifier::REVERSED))
    } else {
        item
    }
}

/// Clamp a stored selection index against the current list length.
/// Lists shrink between snapshots; the cursor stays on the last row
/// rather than pointing past the end. `None` when the list is empty.
pub fn clamped_selection(index: usize, len: usize) -> Option<usize> {
    if len == 0 {
        None
    } else {
        Some(index.min(len - 1))
    }
}

/// Move a selection index by one step, clamped to `[0, len)`.
pub fn step_selection(index: usize, down: bool, len: usize) -> usize {
    let Some(current) = clamped_selection(index, len) else {
        return 0;
    };
    if down {
        (current + 1).min(len - 1)
    } else {
        current.saturating_sub(1)
    }
}

/// Truncate `s` to at most `max_chars` Unicode scalar values, appending
/// `"..."` (three ASCII dots) when cut. When `max_chars <= 3` there is
/// no room for the three-dot marker, so the first `max_chars` characters
/// are returned without any suffix.
pub fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    if max_chars <= 3 {
        return s.chars().take(max_chars).collect();
    }
    let mut out: String = s.chars().take(max_chars - 3).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::Paragraph;
    use ratatui::Terminal;

    #[test]
    fn clamped_selection_handles_empty_and_shrunk_lists() {
        assert_eq!(clamped_selection(0, 0), None);
        assert_eq!(clamped_selection(5, 3), Some(2), "shrunk list clamps");
        assert_eq!(clamped_selection(1, 3), Some(1));
    }

    #[test]
    fn step_selection_clamps_at_both_ends() {
        assert_eq!(step_selection(0, false, 3), 0, "up at top stays");
        assert_eq!(step_selection(2, true, 3), 2, "down at bottom stays");
        assert_eq!(step_selection(0, true, 3), 1);
        assert_eq!(step_selection(2, false, 3), 1);
        assert_eq!(step_selection(7, true, 3), 2, "stale index clamps first");
        assert_eq!(step_selection(0, true, 0), 0, "empty list is safe");
    }

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
    fn truncate_ellipsis_short_string_is_unchanged() {
        assert_eq!(super::truncate_with_ellipsis("hello", 10), "hello");
        assert_eq!(super::truncate_with_ellipsis("hello", 5), "hello");
    }

    #[test]
    fn truncate_ellipsis_long_string_ends_with_three_ascii_dots() {
        let result = super::truncate_with_ellipsis("hello world", 8);
        assert_eq!(result, "hello...");
        assert!(
            result.ends_with("..."),
            "must end with '...', got: {result}"
        );
        assert_eq!(result.chars().count(), 8);
    }

    #[test]
    fn truncate_ellipsis_zero_width_returns_empty() {
        assert_eq!(super::truncate_with_ellipsis("hello", 0), "");
    }

    #[test]
    fn truncate_ellipsis_narrow_width_no_dots() {
        assert_eq!(super::truncate_with_ellipsis("hello", 1), "h");
        assert_eq!(super::truncate_with_ellipsis("hello", 3), "hel");
    }

    #[test]
    fn truncate_ellipsis_four_chars_has_one_visible_plus_dots() {
        assert_eq!(super::truncate_with_ellipsis("hello", 4), "h...");
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
