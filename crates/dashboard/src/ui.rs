//! UI rendering for the dashboard.

use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::app::{App, FocusPanel, SortOrder};

/// RFC3339 timestamp display length (YYYY-MM-DDTHH:MM:SS).
const TIMESTAMP_DISPLAY_LEN: usize = 19;

/// Returns border style based on focus state.
fn focused_border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    }
}

/// Draw the dashboard UI.
pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(10),   // Main content
            Constraint::Length(3), // Footer
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_main(f, app, chunks[1]);
    draw_footer(f, app, chunks[2]);

    if app.show_help {
        draw_help_overlay(f);
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let text = format!(
        " skrills dashboard | Skills: {} | Valid: {} | Invalid: {} | Last: {}",
        app.total_skills,
        app.valid_skills,
        app.invalid_skills,
        if app.last_refresh.is_empty() {
            "-"
        } else {
            &app.last_refresh[..TIMESTAMP_DISPLAY_LEN.min(app.last_refresh.len())]
        }
    );

    let header = Paragraph::new(text)
        .style(Style::default().fg(Color::Cyan).bold())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        );

    f.render_widget(header, area);
}

fn draw_main(f: &mut Frame, app: &mut App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40), // Skills list
            Constraint::Percentage(60), // Right panel
        ])
        .split(area);

    draw_skills_panel(f, app, chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(50), // Activity
            Constraint::Percentage(50), // Metrics
        ])
        .split(chunks[1]);

    draw_activity_panel(f, app, right_chunks[0]);
    draw_metrics_panel(f, app, right_chunks[1]);
}

fn draw_skills_panel(f: &mut Frame, app: &mut App, area: Rect) {
    let border_style = focused_border_style(app.focus == FocusPanel::Skills);

    let visible = app.visible_skill_count();
    let items: Vec<ListItem> = app
        .skills
        .iter()
        .take(visible)
        .map(|skill| {
            let status = match skill.valid {
                Some(true) => "[OK]",
                Some(false) => "[ERR]",
                None => "[--]",
            };

            let style = match skill.valid {
                Some(true) => Style::default().fg(Color::Green),
                Some(false) => Style::default().fg(Color::Red),
                None => Style::default().fg(Color::Gray),
            };

            let line = format!("{} {}", status, skill.name);
            ListItem::new(line).style(style)
        })
        .collect();

    let sort_tag = match app.sort_order {
        SortOrder::Discovery => "",
        SortOrder::Alphabetical => " [A-Z]",
    };
    let title = if visible < app.skills.len() {
        format!(" Skills{} ({}/{}) ", sort_tag, visible, app.skills.len())
    } else {
        format!(" Skills{} ", sort_tag)
    };

    let list = List::new(items)
        .highlight_style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(border_style),
        );

    f.render_stateful_widget(list, area, &mut app.skill_list_state);
}

fn draw_activity_panel(f: &mut Frame, app: &App, area: Rect) {
    let border_style = focused_border_style(app.focus == FocusPanel::Activity);

    // Available width inside the bordered block (minus 2 for left+right borders)
    let inner_width = area.width.saturating_sub(2) as usize;

    let items: Vec<ListItem> = app
        .activity
        .iter()
        .take(area.height.saturating_sub(2) as usize)
        .map(|entry| {
            let style = if entry.message.contains("[ERR]") || entry.message.contains("FAIL") {
                Style::default().fg(Color::Red)
            } else if entry.message.contains("[OK]") || entry.message.contains("PASS") {
                Style::default().fg(Color::Green)
            } else if entry.message.contains("[SYNC]") {
                Style::default().fg(Color::Blue)
            } else {
                Style::default().fg(Color::Gray)
            };
            ListItem::new(entry.format(inner_width)).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(" Activity ")
            .borders(Borders::ALL)
            .border_style(border_style),
    );

    f.render_widget(list, area);
}

fn draw_metrics_panel(f: &mut Frame, app: &App, area: Rect) {
    let border_style = focused_border_style(app.focus == FocusPanel::Metrics);

    let content = if let Some(skill) = app.skills.get(app.skill_index) {
        let mut lines = vec![format!("Skill: {}", skill.name)];

        // Show stats if available
        if let Some(stats) = &app.selected_stats {
            let success_rate = if stats.total_invocations > 0 {
                (stats.successful_invocations as f64 / stats.total_invocations as f64) * 100.0
            } else {
                0.0
            };
            lines.push(format!("Invocations: {}", stats.total_invocations));
            lines.push(format!("Success Rate: {:.1}%", success_rate));
            lines.push(format!("Avg Duration: {:.1}ms", stats.avg_duration_ms));
            lines.push(format!("Total Tokens: {}", stats.total_tokens));
        } else {
            lines.push(format!("Invocations: {}", skill.invocations));
        }

        // Show all discovered locations
        lines.push(String::new());
        lines.push(format!("Locations ({})", skill.locations.len()));
        for (i, loc) in skill.locations.iter().enumerate() {
            lines.push(format!("  {}. [{}] {}", i + 1, loc.source, loc.path));
        }

        lines.join("\n")
    } else {
        "No skill selected".to_string()
    };

    let paragraph = Paragraph::new(content).wrap(Wrap { trim: true }).block(
        Block::default()
            .title(" Skill Info ")
            .borders(Borders::ALL)
            .border_style(border_style),
    );

    f.render_widget(paragraph, area);
}

fn draw_footer(f: &mut Frame, _app: &App, area: Rect) {
    let text = " q:Quit | Tab:Switch Panel | j/k:Navigate | s:Sort | ?:Help ";

    let footer = Paragraph::new(text)
        .style(Style::default().fg(Color::DarkGray))
        .block(Block::default().borders(Borders::ALL));

    f.render_widget(footer, area);
}

fn draw_help_overlay(f: &mut Frame) {
    let area = centered_rect(60, 50, f.area());

    let help_text = r#"
Keyboard Shortcuts:

  q, Esc     Quit dashboard
  Tab        Switch to next panel
  Shift+Tab  Switch to previous panel
  j, Down    Select next item
  k, Up      Select previous item
  Home       Jump to first item
  End        Jump to last item
  s          Toggle sort (discovery / A-Z)
  ?          Toggle this help

Panels:
  Skills     List of discovered skills
  Activity   Recent events and actions
  Metrics    Stats for selected skill
"#;

    // Clear the overlay area first so background content doesn't bleed through
    f.render_widget(Clear, area);

    let paragraph = Paragraph::new(help_text)
        .style(Style::default().fg(Color::White).bg(Color::Black))
        .block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .style(Style::default().bg(Color::Black)),
        );

    f.render_widget(paragraph, area);
}

/// Create a centered rect.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
