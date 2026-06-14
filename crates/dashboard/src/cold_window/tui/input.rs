//! Keystroke routing for the cold-window TUI.
//!
//! [`handle_key`] is the single entry point: it resolves overlays,
//! globals, and pane keymaps in priority order (FR-4) and reports
//! whether the loop should redraw or quit.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::UiState;
use crate::cold_window::overlay::Overlay;
use crate::cold_window::{
    AlertPane, ColdWindowState, FocusTarget, HintPane, HintPaneState, ResearchPane,
    ResearchPaneState,
};

/// What the event loop should do after a keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyOutcome {
    /// Tear down the TUI and return.
    Quit,
    /// Re-render with the (possibly mutated) pane state.
    Redraw,
}

/// Route a keystroke to the interface state and panes.
///
/// Routing order (FR-4):
///
/// 1. `Ctrl-C` always quits.
/// 2. `q` closes the topmost overlay; at the base surface it quits.
/// 3. `Esc` closes the topmost overlay; at the base surface it does
///    nothing (BREAKING since 0.8.x: `Esc` no longer quits).
/// 4. An open overlay consumes every other key.
/// 5. Globals: `Tab`/`Shift-Tab` move focus.
/// 6. Everything else is forwarded to all three pane handlers; their
///    keybindings are disjoint (`A`/`d` alerts, `0`-`5`/`P` hints,
///    `R` research), so focus does not gate them (FR-1.4): focus
///    governs only what the hint bar describes and what `Enter`/`z`
///    target.
pub fn handle_key(
    key: KeyEvent,
    ui: &mut UiState,
    snap_state: &mut ColdWindowState,
    hint_state: &mut HintPaneState,
    research_state: &mut ResearchPaneState,
) -> KeyOutcome {
    let ctrl_c = key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'));
    if ctrl_c {
        return KeyOutcome::Quit;
    }

    // The palette is a text-input surface: it must see raw characters
    // (including `q` and `?`) before any global binding fires.
    if matches!(ui.overlays.top(), Some(Overlay::Palette { .. })) {
        return handle_palette_key(key.code, ui, snap_state, hint_state, research_state);
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => {
            return if ui.overlays.pop().is_some() {
                KeyOutcome::Redraw
            } else {
                KeyOutcome::Quit
            };
        }
        KeyCode::Esc => {
            // Pops the topmost overlay; with none open it unzooms;
            // harmless no-op at the bare base surface.
            if ui.overlays.pop().is_none() {
                ui.zoomed = false;
            }
            return KeyOutcome::Redraw;
        }
        _ => {}
    }

    // `?` toggles help: opens it at the base, closes it when help is
    // already the topmost overlay (FR-3.1, FR-3.3).
    if key.code == KeyCode::Char('?') {
        if matches!(ui.overlays.top(), Some(Overlay::Help)) {
            let _ = ui.overlays.pop();
        } else {
            ui.overlays.push(Overlay::Help);
        }
        return KeyOutcome::Redraw;
    }

    // `:` opens the command palette (the novice-expert bridge; k9s
    // pattern). The palette branch above then owns the keyboard.
    if key.code == KeyCode::Char(':') {
        ui.overlays.push(Overlay::Palette {
            query: String::new(),
            selected: 0,
        });
        return KeyOutcome::Redraw;
    }

    // The topmost overlay holds the keyboard: pane keys must not leak
    // underneath it (FR-3.3, FR-4.1).
    if !ui.overlays.is_empty() {
        return KeyOutcome::Redraw;
    }

    match key.code {
        KeyCode::Tab => {
            ui.focus = ui.focus.next();
            return KeyOutcome::Redraw;
        }
        KeyCode::BackTab => {
            ui.focus = ui.focus.prev();
            return KeyOutcome::Redraw;
        }
        // Zoom the focused pane to the full body (FR-5.2).
        KeyCode::Char('z') => {
            ui.zoomed = !ui.zoomed;
            return KeyOutcome::Redraw;
        }
        // Drill into the focused pane's selected item (FR-5.1).
        KeyCode::Enter => {
            if let Some(detail) = detail_overlay(ui, snap_state, hint_state) {
                ui.overlays.push(detail);
            }
            return KeyOutcome::Redraw;
        }
        // Selection moves within the focused pane only (FR-5).
        KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
            let down = matches!(key.code, KeyCode::Down | KeyCode::Char('j'));
            let len = focused_list_len(ui.focus, snap_state, hint_state);
            let cursor = ui.selected.for_focus_mut(ui.focus);
            *cursor = crate::cold_window::focus::step_selection(*cursor, down, len);
            return KeyOutcome::Redraw;
        }
        _ => {}
    }

    // Disjoint keymaps: forwarding the same code to each handler is
    // safe because at most one will act on it.
    let _ = AlertPane::handle_key(snap_state, key.code);
    let _ = HintPane::handle_key(snap_state, hint_state, key.code);
    let _ = ResearchPane::handle_key(snap_state, research_state, key.code);
    KeyOutcome::Redraw
}

/// Length of the list the focused pane is showing, for selection
/// clamping. The research pane counts its findings whether or not the
/// pane is expanded (the cursor is simply invisible while collapsed).
fn focused_list_len(
    focus: FocusTarget,
    snap_state: &ColdWindowState,
    hint_state: &HintPaneState,
) -> usize {
    match focus {
        FocusTarget::Alerts => snap_state.visible_alerts().len(),
        FocusTarget::Hints => hint_state.visible_hints(snap_state).len(),
        FocusTarget::Research => snap_state
            .current
            .as_deref()
            .map(|s| s.research_findings.len())
            .unwrap_or(0),
    }
}

/// Keystroke routing while the command palette is the topmost overlay.
///
/// Characters edit the query, `Up`/`Down` move the selection within
/// the filtered list, `Enter` closes the palette and replays the
/// selected command's key through [`handle_key`] (so palette execution
/// can never drift from what the key itself does), and `Esc` closes
/// without running anything.
fn handle_palette_key(
    code: KeyCode,
    ui: &mut UiState,
    snap_state: &mut ColdWindowState,
    hint_state: &mut HintPaneState,
    research_state: &mut ResearchPaneState,
) -> KeyOutcome {
    use crate::cold_window::focus::{clamped_selection, step_selection};
    use crate::cold_window::keymap::palette_matches;

    // Take the palette off the stack, mutate, and decide whether to
    // put it back; avoids aliasing the stack while editing its top.
    let Some(Overlay::Palette {
        mut query,
        mut selected,
    }) = ui.overlays.pop()
    else {
        return KeyOutcome::Redraw;
    };

    match code {
        KeyCode::Esc => KeyOutcome::Redraw, // closed, nothing run
        KeyCode::Enter => {
            let matches = palette_matches(&query);
            let Some(index) = clamped_selection(selected, matches.len()) else {
                // No matching command: keep the palette open so the
                // user can fix the query.
                ui.overlays.push(Overlay::Palette { query, selected });
                return KeyOutcome::Redraw;
            };
            let replay = matches[index].code;
            handle_key(
                KeyEvent::new(replay, KeyModifiers::NONE),
                ui,
                snap_state,
                hint_state,
                research_state,
            )
        }
        KeyCode::Up | KeyCode::Down => {
            let len = palette_matches(&query).len();
            selected = step_selection(selected, code == KeyCode::Down, len);
            ui.overlays.push(Overlay::Palette { query, selected });
            KeyOutcome::Redraw
        }
        KeyCode::Backspace => {
            query.pop();
            selected = 0;
            ui.overlays.push(Overlay::Palette { query, selected });
            KeyOutcome::Redraw
        }
        KeyCode::Char(c) => {
            query.push(c);
            selected = 0;
            ui.overlays.push(Overlay::Palette { query, selected });
            KeyOutcome::Redraw
        }
        _ => {
            ui.overlays.push(Overlay::Palette { query, selected });
            KeyOutcome::Redraw
        }
    }
}

/// Build the drill-down detail overlay for the focused pane's selected
/// item (FR-5.1). `None` when the focused list is empty, so `Enter`
/// degrades to a no-op instead of opening a blank popup.
fn detail_overlay(
    ui: &UiState,
    snap_state: &ColdWindowState,
    hint_state: &HintPaneState,
) -> Option<Overlay> {
    let index = ui.selected.for_focus(ui.focus);
    match ui.focus {
        FocusTarget::Alerts => {
            let visible = snap_state.visible_alerts();
            let alert = visible.get(crate::cold_window::focus::clamped_selection(
                index,
                visible.len(),
            )?)?;
            Some(Overlay::Detail {
                title: alert.title.clone(),
                lines: vec![
                    format!("severity:    {}", alert.severity.short_label()),
                    format!("fingerprint: {}", alert.fingerprint),
                    format!("fired at:    {} ms", alert.fired_at_ms),
                    format!("dwell ticks: {}", alert.dwell_ticks),
                    String::new(),
                    alert.message.clone(),
                ],
            })
        }
        FocusTarget::Hints => {
            let visible = hint_state.visible_hints(snap_state);
            let hint = visible.get(crate::cold_window::focus::clamped_selection(
                index,
                visible.len(),
            )?)?;
            Some(Overlay::Detail {
                title: hint.hint.uri.clone(),
                lines: vec![
                    format!("category:  {}", hint.hint.category.label()),
                    format!("score:     {:.1}", hint.score),
                    format!("frequency: {}", hint.hint.frequency),
                    format!("impact:    {:.1}", hint.hint.impact),
                    format!("ease:      {:.1}", hint.hint.ease_score),
                    format!("age:       {:.1} days", hint.hint.age_days),
                    String::new(),
                    hint.hint.message.clone(),
                ],
            })
        }
        FocusTarget::Research => {
            let snap = snap_state.current.as_deref()?;
            let findings = &snap.research_findings;
            let finding = findings.get(crate::cold_window::focus::clamped_selection(
                index,
                findings.len(),
            )?)?;
            Some(Overlay::Detail {
                title: finding.title.clone(),
                lines: vec![
                    format!("channel:    {}", finding.channel.short_label()),
                    format!("score:      {:.1}", finding.score),
                    format!("fetched at: {} ms", finding.fetched_at_ms),
                    String::new(),
                    finding.url.clone(),
                ],
            })
        }
    }
}
