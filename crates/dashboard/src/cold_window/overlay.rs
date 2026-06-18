//! Modal overlay stack for the cold-window TUI.
//!
//! All modal surfaces (help, drill-down detail, future config popups)
//! live on a single `Vec`-backed stack. The topmost overlay consumes
//! every keystroke; `Esc` pops it (and does nothing at the base
//! surface), `q` pops it too but quits once the stack is empty. The
//! base panes therefore never need to know whether an overlay is open.
//!
//! Overlays render last, over the panes, into a centered rect cleared
//! with [`Clear`] (the gitui `popup_stack` pattern; in-tree per NFR-2
//! rather than a `tui-popup` dependency).

use ratatui::layout::Rect;
use ratatui::prelude::*;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use super::focus::FocusTarget;

/// One modal surface on the stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Overlay {
    /// Keybinding help (`?`), grouped by scope with the focused pane's
    /// section first.
    Help,
    /// Drill-down detail for one selected item (`Enter`).
    Detail {
        /// Popup title (e.g. the alert title or hint URI).
        title: String,
        /// Pre-formatted body lines.
        lines: Vec<String>,
    },
    /// The `:` command palette: type to filter, `Enter` runs the
    /// selected command by replaying its key (k9s pattern, TR-006).
    Palette {
        /// Current filter text.
        query: String,
        /// Cursor into the filtered command list.
        selected: usize,
    },
}

/// LIFO stack of modal overlays. Empty stack means the base surface
/// has the keyboard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverlayStack {
    items: Vec<Overlay>,
}

impl OverlayStack {
    /// True when no overlay is open.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Open `overlay` on top of the stack.
    pub fn push(&mut self, overlay: Overlay) {
        self.items.push(overlay);
    }

    /// Close the topmost overlay; returns it, or `None` at the base.
    pub fn pop(&mut self) -> Option<Overlay> {
        self.items.pop()
    }

    /// The overlay currently holding the keyboard, if any.
    pub fn top(&self) -> Option<&Overlay> {
        self.items.last()
    }
}

/// Centered popup rect: 80% of the frame, clamped to a 72x20 maximum
/// and never exceeding the frame itself (tiny terminals get the whole
/// frame rather than a clipped or out-of-bounds rect).
pub fn popup_rect(frame: Rect) -> Rect {
    let w = (u32::from(frame.width) * 8 / 10).min(72) as u16;
    let h = (u32::from(frame.height) * 8 / 10).min(20) as u16;
    let w = w.clamp(1, frame.width).max(frame.width.min(10));
    let h = h.clamp(1, frame.height).max(frame.height.min(3));
    let x = frame.x + (frame.width - w) / 2;
    let y = frame.y + (frame.height - h) / 2;
    Rect::new(x, y, w, h)
}

/// Render the topmost overlay (if any) over the already-drawn panes.
pub fn render(stack: &OverlayStack, focus: FocusTarget, frame: &mut Frame<'_>) {
    let Some(top) = stack.top() else { return };
    let area = popup_rect(frame.area());
    frame.render_widget(Clear, area);
    match top {
        Overlay::Help => render_help(focus, frame, area),
        Overlay::Detail { title, lines } => render_detail(title, lines, frame, area),
        Overlay::Palette { query, selected } => render_palette(query, *selected, frame, area),
    }
}

fn render_palette(query: &str, selected: usize, frame: &mut Frame<'_>, area: Rect) {
    use super::focus::clamped_selection;
    use super::keymap::palette_matches;

    let matches = palette_matches(query);
    let cursor = clamped_selection(selected, matches.len());
    let mut lines = vec![Line::from(vec![
        Span::styled(" : ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(query.to_string()),
        Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ])];
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "   no matching command",
            Style::default().fg(Color::DarkGray),
        )));
    }
    for (i, entry) in matches.iter().enumerate() {
        let marker = if cursor == Some(i) { " > " } else { "   " };
        let style = if cursor == Some(i) {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{}", entry.label),
            style,
        )));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().add_modifier(Modifier::BOLD))
        .title(" Commands (Enter run, Esc close) ");
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_help(focus: FocusTarget, frame: &mut Frame<'_>, area: Rect) {
    let lines = help_lines(focus);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().add_modifier(Modifier::BOLD))
        .title(" Help (Esc to close) ");
    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Help body: bindings grouped by scope, the focused pane's section
/// first (FR-3.2). Content comes from the keymap table, the single
/// source of truth.
fn help_lines(focus: FocusTarget) -> Vec<Line<'static>> {
    use super::keymap::{bindings_for, BindingScope};

    let focused_scope = match focus {
        FocusTarget::Alerts => BindingScope::Alerts,
        FocusTarget::Hints => BindingScope::Hints,
        FocusTarget::Research => BindingScope::Research,
    };
    let mut order = vec![
        focused_scope,
        BindingScope::Alerts,
        BindingScope::Hints,
        BindingScope::Research,
        BindingScope::Global,
    ];
    order.dedup();
    // Remove later duplicates of the focused scope.
    let order: Vec<BindingScope> = {
        let mut seen = Vec::new();
        order.retain(|s| {
            if seen.contains(s) {
                false
            } else {
                seen.push(*s);
                true
            }
        });
        order
    };

    let mut lines = Vec::new();
    for scope in order {
        let name = match scope {
            BindingScope::Global => "Global",
            BindingScope::Alerts => "Alerts",
            BindingScope::Hints => "Hints",
            BindingScope::Research => "Research",
        };
        lines.push(Line::from(Span::styled(
            format!(" {name}"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )));
        for b in bindings_for(scope) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("   {:<9}", b.keys),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw(b.action),
            ]));
        }
    }
    lines
}

fn render_detail(title: &str, lines: &[String], frame: &mut Frame<'_>, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().add_modifier(Modifier::BOLD))
        .title(format!(" {title} (Esc to close) "));
    let body: Vec<Line<'_>> = lines.iter().map(|l| Line::from(l.as_str())).collect();
    frame.render_widget(
        Paragraph::new(body).block(block).wrap(Wrap { trim: false }),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn stack_is_lifo_and_reports_emptiness() {
        let mut stack = OverlayStack::default();
        assert!(stack.is_empty());
        assert_eq!(stack.pop(), None, "pop at base is a no-op");
        stack.push(Overlay::Help);
        stack.push(Overlay::Detail {
            title: "t".into(),
            lines: vec![],
        });
        assert!(!stack.is_empty());
        assert!(matches!(stack.top(), Some(Overlay::Detail { .. })));
        assert!(matches!(stack.pop(), Some(Overlay::Detail { .. })));
        assert_eq!(stack.pop(), Some(Overlay::Help));
        assert!(stack.is_empty());
    }

    #[test]
    fn popup_rect_never_exceeds_the_frame() {
        for (w, h) in [(1u16, 1u16), (5, 2), (40, 12), (120, 40), (300, 100)] {
            let frame = Rect::new(0, 0, w, h);
            let r = popup_rect(frame);
            assert!(r.width >= 1 && r.height >= 1, "{w}x{h}: degenerate rect");
            assert!(
                r.x + r.width <= w && r.y + r.height <= h,
                "{w}x{h}: popup {r:?} exceeds frame"
            );
        }
    }

    #[test]
    fn render_overlay_overwrites_pane_cells() {
        // FR-4.3: the overlay must visually cover whatever the panes
        // drew underneath it.
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        let mut stack = OverlayStack::default();
        stack.push(Overlay::Detail {
            title: "DETAIL-TITLE".into(),
            lines: vec!["detail-body-line".into()],
        });
        terminal
            .draw(|f| {
                // Underlay: fill the frame with a sentinel.
                let filler = Paragraph::new(vec![Line::from("#".repeat(60)); 20]);
                f.render_widget(filler, f.area());
                render(&stack, FocusTarget::Alerts, f);
            })
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("DETAIL-TITLE"), "overlay title missing");
        assert!(text.contains("detail-body-line"), "overlay body missing");
    }

    #[test]
    fn help_lists_focused_pane_section_first() {
        // FR-3.2: the focused scope's section heads the help body.
        let lines = help_lines(FocusTarget::Hints);
        let first = lines.first().expect("help has content");
        let first_text: String = first.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            first_text.contains("Hints"),
            "focused scope must come first, got: {first_text}"
        );
        // And every scope appears exactly once.
        let joined: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref().to_string()))
            .collect();
        for name in ["Global", "Alerts", "Hints", "Research"] {
            assert_eq!(
                joined.matches(name).count(),
                1,
                "{name} section must appear exactly once"
            );
        }
    }
}
