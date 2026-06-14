//! Composite, layout, and key-routing tests for the cold-window TUI
//! shell. These exercise the public entry points ([`draw`],
//! [`handle_key`]) and the private tier planners together against a
//! `TestBackend`, so they live with the shell rather than any one
//! submodule.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use skrills_snapshot::{
    Alert, AlertBand, Hint, HintCategory, LoadSample, ResearchChannel, ResearchFinding,
    ResearchQuota, ScoredHint, Severity, TokenEntry, TokenLedger, WindowSnapshot,
};

use super::input::{handle_key, KeyOutcome};
use super::layout::{layout_mode, plan_for, plan_layout, plan_layout_with, LayoutMode};
use super::render::draw;
use super::UiState;
use crate::cold_window::{ColdWindowState, FocusTarget, HintPaneState, Overlay, ResearchPaneState};

fn rich_snapshot() -> Arc<WindowSnapshot> {
    Arc::new(WindowSnapshot {
        version: 7,
        timestamp_ms: 1_700_000_000_000,
        token_ledger: TokenLedger {
            per_skill: vec![TokenEntry {
                source: "skill://demo".into(),
                tokens: 42_000,
            }],
            per_plugin: vec![],
            per_mcp: vec![],
            conversation_cache_reads: 0,
            conversation_cache_writes: 0,
            total: 42_000,
        },
        alerts: vec![Alert {
            fingerprint: "w1".into(),
            severity: Severity::Warning,
            title: "budget pressure".into(),
            message: "token total climbing".into(),
            band: Some(AlertBand::new(0.0, 0.0, 1.0, 0.95).expect("band")),
            fired_at_ms: 1_700_000_000_000,
            dwell_ticks: 2,
        }],
        hints: vec![ScoredHint {
            hint: Hint {
                uri: "skill://refactor".into(),
                category: HintCategory::Token,
                message: "split large skill".into(),
                frequency: 3,
                impact: 8.5,
                ease_score: 6.0,
                age_days: 1.0,
            },
            score: 0.9,
            pinned: false,
        }],
        research_findings: vec![],
        plugin_health: vec![],
        load_sample: LoadSample::default(),
        next_tick_ms: 2_000,
    })
}

#[test]
fn draw_composes_all_panes_without_panic() {
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(rich_snapshot());
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();
    let ui = UiState::new();
    terminal
        .draw(|f| {
            draw(
                f,
                &ui,
                &snap_state,
                &hint_state,
                &research_state,
                Some(ResearchQuota::new(3, 10)),
                100_000,
            )
        })
        .expect("composite draw must not fail");
}

#[test]
fn draw_survives_tiny_and_huge_areas() {
    // R9: the composite must not panic across terminal sizes, with
    // research both collapsed and expanded. The expanded pass at
    // `(44, 4)` drives the height-starved expanded-Narrow arm
    // (`Percentage(35/30/35)`) the collapsed default never reaches.
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(rich_snapshot());
    let hint_state = HintPaneState::new();
    let ui = UiState::new();
    let collapsed = ResearchPaneState::default();
    let mut expanded = ResearchPaneState::new();
    expanded.collapsed = false;
    for research_state in [&collapsed, &expanded] {
        for (w, h) in [(20u16, 5u16), (40, 12), (200, 60), (8, 2), (44, 4)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| {
                    draw(
                        f,
                        &ui,
                        &snap_state,
                        &hint_state,
                        research_state,
                        None,
                        100_000,
                    )
                })
                .unwrap_or_else(|e| {
                    panic!(
                        "draw panicked at {w}x{h} (collapsed={}): {e}",
                        research_state.collapsed
                    )
                });
        }
    }
}

#[test]
fn first_paint_reflects_snapshot_under_500ms() {
    // SC3: startup-to-first-snapshot must beat 500 ms. The launch
    // path's cost is dominated by ingest and a single composite
    // render; we measure exactly that against a real backend.
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();
    let ui = UiState::new();

    let t0 = Instant::now();
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(rich_snapshot());
    terminal
        .draw(|f| {
            draw(
                f,
                &ui,
                &snap_state,
                &hint_state,
                &research_state,
                Some(ResearchQuota::new(0, 10)),
                100_000,
            )
        })
        .unwrap();
    let elapsed = t0.elapsed();

    assert_eq!(snap_state.token_total(), 42_000, "snapshot not ingested");
    assert!(
        elapsed < Duration::from_millis(500),
        "SC3 first paint took {elapsed:?}, exceeds 500 ms budget"
    );
}

// --- Responsive layout (R9: adapt to terminal size) -------------

fn area(w: u16, h: u16) -> Rect {
    Rect::new(0, 0, w, h)
}

#[test]
fn mode_thresholds_map_width_to_tier() {
    assert_eq!(layout_mode(area(80, 24)), LayoutMode::Wide);
    assert_eq!(layout_mode(area(120, 40)), LayoutMode::Wide);
    assert_eq!(layout_mode(area(79, 24)), LayoutMode::Medium);
    assert_eq!(layout_mode(area(60, 24)), LayoutMode::Medium);
    assert_eq!(layout_mode(area(59, 24)), LayoutMode::Narrow);
    assert_eq!(layout_mode(area(45, 24)), LayoutMode::Narrow);
    assert_eq!(layout_mode(area(44, 24)), LayoutMode::Compact);
    assert_eq!(layout_mode(area(40, 30)), LayoutMode::Compact);
}

#[test]
fn status_bar_is_always_one_line_at_the_bottom() {
    for (w, h) in [(120u16, 40u16), (70, 30), (44, 30)] {
        let l = plan_layout(area(w, h), true);
        assert_eq!(l.status.height, 1, "{w}x{h}: status must be one row");
        assert_eq!(l.status.width, w, "{w}x{h}: status spans full width");
        assert_eq!(
            l.status.y,
            h - 1,
            "{w}x{h}: status pinned to the bottom row"
        );
    }
}

#[test]
fn wide_and_medium_are_columnar() {
    for mode in [LayoutMode::Wide, LayoutMode::Medium] {
        let l = plan_for(mode, area(100, 40), true);
        // Research sits to the right of the alert/hint column.
        assert!(
            l.research.x > l.alerts.x,
            "{mode:?}: research must be the right column"
        );
        // Alerts stack above hints in a shared left column.
        assert_eq!(l.alerts.x, l.hints.x, "{mode:?}: left column shared");
        assert!(l.alerts.y < l.hints.y, "{mode:?}: alerts above hints");
    }
}

#[test]
fn medium_gives_left_panes_more_room_than_wide() {
    // Width 70 is inside the Medium band (60..=79), so the 68%
    // left-column constant is exercised at a size Medium actually
    // occupies, not at 100, which `layout_mode` routes to Wide.
    let wide = plan_for(LayoutMode::Wide, area(70, 40), true);
    let medium = plan_for(LayoutMode::Medium, area(70, 40), true);
    assert!(
        medium.alerts.width > wide.alerts.width,
        "medium left column ({}) should be wider than wide ({})",
        medium.alerts.width,
        wide.alerts.width
    );
    assert!(
        medium.research.width < wide.research.width,
        "medium research ({}) should be slimmer than wide ({})",
        medium.research.width,
        wide.research.width
    );
}

#[test]
fn research_collapsed_only_affects_the_narrow_stack() {
    // `plan_layout`'s contract says `research_collapsed` reshapes
    // only the Narrow vertical stack. Pin it: the two columnar tiers
    // must produce byte-identical layouts regardless of the flag.
    for mode in [LayoutMode::Wide, LayoutMode::Medium] {
        assert_eq!(
            plan_for(mode, area(100, 40), true),
            plan_for(mode, area(100, 40), false),
            "{mode:?}: research_collapsed must not change the columnar layout"
        );
    }
    // ...and it *does* change the Narrow stack, so the assertion
    // above is guarding a real distinction, not a constant.
    assert_ne!(
        plan_for(LayoutMode::Narrow, area(44, 30), true),
        plan_for(LayoutMode::Narrow, area(44, 30), false),
        "Narrow: research_collapsed must reshape the stack"
    );
}

#[test]
fn narrow_stacks_panes_in_a_single_full_width_column() {
    let w = 50;
    let l = plan_layout(area(w, 30), true);
    for (name, r) in [
        ("alerts", l.alerts),
        ("hints", l.hints),
        ("research", l.research),
    ] {
        assert_eq!(r.x, 0, "{name} must start at the left edge");
        assert_eq!(r.width, w, "{name} must span the full width");
    }
    assert!(l.alerts.y < l.hints.y, "alerts above hints");
    assert!(l.hints.y < l.research.y, "hints above research");
}

#[test]
fn narrow_research_grows_when_expanded() {
    let collapsed = plan_layout(area(50, 30), true);
    let expanded = plan_layout(area(50, 30), false);
    assert_eq!(
        collapsed.research.height, 3,
        "collapsed research is a 3-line badge"
    );
    assert!(
        expanded.research.height > collapsed.research.height,
        "expanded research ({}) should be taller than collapsed ({})",
        expanded.research.height,
        collapsed.research.height
    );
}

/// Flatten a rendered `TestBackend` buffer into one plain string so
/// tests can assert on visible text without caring about cell
/// coordinates.
fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

/// `rich_snapshot` with a single research finding attached, so the
/// expanded research pane has a row to render.
fn snapshot_with_research() -> Arc<WindowSnapshot> {
    let mut snap = (*rich_snapshot()).clone();
    snap.research_findings = vec![ResearchFinding {
        fingerprint: "fp1".into(),
        channel: ResearchChannel::GitHub,
        title: "responsive layout".into(),
        url: "https://example.com/fp1".into(),
        score: 9.0,
        fetched_at_ms: 0,
    }];
    Arc::new(snap)
}

#[test]
fn draw_renders_expanded_research_in_narrow_and_medium_tiers() {
    // The pure-function tests pin the *geometry* of the Medium tier
    // and the narrow expanded-research arm, but neither is ever
    // pushed through the real `ResearchPane::render`. A regression
    // that handed an expanded pane the collapsed badge's 3-line slot
    // would satisfy the geometry tests yet truncate the live render.
    // Drive both tiers through `draw` to guard the seam.
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(snapshot_with_research());
    let hint_state = HintPaneState::new();
    // collapsed: false exercises the Percentage(35/30/35) narrow arm
    // and the expanded render path that lists findings.
    let research_state = ResearchPaneState {
        collapsed: false,
        ..ResearchPaneState::default()
    };
    let ui = UiState::new();

    // Narrow tier (width < 60): expanded research stacks full-width
    // with real vertical room. Its title carries the "findings"
    // marker that the collapsed badge ("press R to expand") never
    // shows, proving the expanded path reached the row renderer.
    let mut narrow = Terminal::new(TestBackend::new(50, 30)).unwrap();
    narrow
        .draw(|f| {
            draw(
                f,
                &ui,
                &snap_state,
                &hint_state,
                &research_state,
                None,
                100_000,
            )
        })
        .expect("narrow expanded draw must not panic");
    assert!(
        buffer_text(&narrow).contains("findings"),
        "expanded research must render its findings list in the narrow stack"
    );

    // Medium tier (60 <= width < 80): the slimmer columnar arm that
    // no `draw` test exercised before. Guard it against renderer
    // panics with the expanded pane in the right column.
    let mut medium = Terminal::new(TestBackend::new(70, 30)).unwrap();
    medium
        .draw(|f| {
            draw(
                f,
                &ui,
                &snap_state,
                &hint_state,
                &research_state,
                None,
                100_000,
            )
        })
        .expect("medium expanded draw must not panic");
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn q_quits_at_base_but_esc_does_not() {
    // BREAKING (FR-4.2): Esc stopped quitting when the overlay
    // stack landed; it only closes overlays now.
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();
    assert_eq!(
        handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r),
        KeyOutcome::Redraw,
        "Esc at the base surface must not quit"
    );
    assert_eq!(
        handle_key(key(KeyCode::Char('q')), &mut ui, &mut s, &mut h, &mut r),
        KeyOutcome::Quit
    );
}

#[test]
fn esc_and_q_pop_overlays_before_anything_else() {
    // FR-4.2: Esc pops; q pops too and only quits at the base.
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();

    ui.overlays.push(Overlay::Help);
    ui.overlays.push(Overlay::Help);
    assert_eq!(
        handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r),
        KeyOutcome::Redraw
    );
    assert_eq!(
        handle_key(key(KeyCode::Char('q')), &mut ui, &mut s, &mut h, &mut r),
        KeyOutcome::Redraw,
        "q with an overlay open closes it instead of quitting"
    );
    assert!(ui.overlays.is_empty(), "both overlays popped");
    assert_eq!(
        handle_key(key(KeyCode::Char('q')), &mut ui, &mut s, &mut h, &mut r),
        KeyOutcome::Quit,
        "q at the base quits"
    );
}

#[test]
fn open_overlay_consumes_pane_and_focus_keys() {
    // FR-4.1: the topmost overlay holds the keyboard; pane state
    // and focus must not change underneath it.
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    s.ingest(rich_snapshot());
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();
    ui.overlays.push(Overlay::Help);

    let warnings_before = s.visible_alerts().len();
    handle_key(key(KeyCode::Char('d')), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(
        s.visible_alerts().len(),
        warnings_before,
        "'d' must not reach the alert pane through an overlay"
    );
    handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(
        ui.focus,
        FocusTarget::Alerts,
        "Tab must not move focus through an overlay"
    );
    handle_key(key(KeyCode::Char('R')), &mut ui, &mut s, &mut h, &mut r);
    assert!(
        r.collapsed,
        "'R' must not toggle the research pane through an overlay"
    );
}

/// Snapshot with two warnings so the alert list has two rows to
/// move a selection across.
fn two_warning_snapshot() -> Arc<WindowSnapshot> {
    let mut snap = (*rich_snapshot()).clone();
    snap.alerts = vec![
        Alert {
            fingerprint: "w-first".into(),
            severity: Severity::Warning,
            title: "first-alert".into(),
            message: "m1".into(),
            band: None,
            fired_at_ms: 200,
            dwell_ticks: 1,
        },
        Alert {
            fingerprint: "w-second".into(),
            severity: Severity::Warning,
            title: "second-alert".into(),
            message: "m2".into(),
            band: None,
            fired_at_ms: 100,
            dwell_ticks: 1,
        },
    ];
    Arc::new(snap)
}

/// One terminal row as trimmed text.
fn row_text(terminal: &Terminal<TestBackend>, y: u16) -> String {
    let buf = terminal.backend().buffer();
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
}

#[test]
fn jk_and_arrows_move_selection_in_the_focused_pane_only() {
    // FR-5/T5: selection keys act on the focused pane's cursor and
    // leave the other panes' cursors untouched.
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    s.ingest(two_warning_snapshot());
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();

    handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.selected.alerts, 1, "j moves the alerts cursor down");
    assert_eq!(ui.selected.hints, 0, "hints cursor untouched");
    handle_key(key(KeyCode::Char('k')), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.selected.alerts, 0, "k moves it back up");
    handle_key(key(KeyCode::Down), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.selected.alerts, 1, "Down mirrors j");
    handle_key(key(KeyCode::Up), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.selected.alerts, 0, "Up mirrors k");

    // Clamping: two items means the cursor never reaches index 2.
    handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.selected.alerts, 1, "cursor clamps at the last row");

    // Focus hints: same keys now drive the hints cursor (one hint
    // in the snapshot, so it stays clamped at 0).
    handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.selected.hints, 0, "single hint clamps at 0");
    assert_eq!(ui.selected.alerts, 1, "alerts cursor persists");
}

#[test]
fn selected_row_carries_a_cursor_marker_visible_without_color() {
    // FR-5/T5: the `> ` row marker must sit on exactly the selected
    // row of the focused pane.
    let mut s = ColdWindowState::new();
    s.ingest(two_warning_snapshot());
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();
    let mut ui = UiState::new();
    ui.selected.alerts = 1;

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|f| draw(f, &ui, &s, &hint_state, &research_state, None, 100_000))
        .unwrap();
    // Alert rows start under the top border: row 1 is the first
    // alert, row 2 the second (selected) one.
    // Rows begin with the pane's left border glyph; the marker (or
    // its two-space gutter) comes immediately after it.
    let first = row_text(&terminal, 1);
    let second = row_text(&terminal, 2);
    assert!(
        second.starts_with("│>"),
        "selected row must carry the > marker after the border, got: {second:?}"
    );
    assert!(
        !first.contains('>'),
        "unselected row must not carry the marker, got: {first:?}"
    );
    assert!(second.contains("second-alert"), "marker on the right row");
}

#[test]
fn compact_tier_draws_only_the_focused_pane() {
    // FR-6.1/FR-6.2: below 45 columns, focus is visibility; the
    // other panes' titles must not appear anywhere on the frame.
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(rich_snapshot());
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();
    let mut ui = UiState::new();
    ui.focus = FocusTarget::Hints;

    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    terminal
        .draw(|f| {
            draw(
                f,
                &ui,
                &snap_state,
                &hint_state,
                &research_state,
                None,
                100_000,
            )
        })
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("> Hints"), "focused pane visible: {text}");
    assert!(
        !text.contains("Alerts  W:"),
        "alert pane must be hidden in compact, got: {text}"
    );
}

#[test]
fn size_guard_replaces_panes_below_the_floor() {
    // FR-6.3: 20x6 is the floor; below it (either dimension) the
    // guard message is the whole frame.
    let snap_state = ColdWindowState::new();
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();
    let ui = UiState::new();

    let render_at = |w: u16, h: u16| -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    &ui,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    None,
                    100_000,
                )
            })
            .unwrap();
        buffer_text(&terminal)
    };

    assert!(
        render_at(19, 10).contains("terminal too small"),
        "width below floor must show the guard"
    );
    assert!(
        render_at(30, 5).contains("terminal too small"),
        "height below floor must show the guard"
    );
    assert!(
        !render_at(20, 6).contains("terminal too small"),
        "exactly at the floor the panes render"
    );
}

#[test]
fn zoom_gives_the_focused_pane_the_full_body_at_every_tier() {
    // FR-5.2/FR-5.3: zoom is a layout-level override, independent
    // of the responsive tier; the status bar stays pinned.
    for (w, h) in [(44u16, 30u16), (70, 30), (120, 40)] {
        let a = area(w, h);
        let l = plan_layout_with(a, true, Some(FocusTarget::Hints));
        assert_eq!(
            l.hints,
            Rect::new(0, 0, w, h - 1),
            "{w}x{h}: zoomed pane must own the full body"
        );
        assert_eq!(l.alerts, Rect::default(), "{w}x{h}: alerts hidden");
        assert_eq!(l.research, Rect::default(), "{w}x{h}: research hidden");
        assert_eq!(l.status.y, h - 1, "{w}x{h}: status stays pinned");
    }
}

#[test]
fn z_toggles_zoom_and_esc_unzooms_only_with_no_overlay() {
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();

    handle_key(key(KeyCode::Char('z')), &mut ui, &mut s, &mut h, &mut r);
    assert!(ui.zoomed, "z zooms");

    // An open overlay absorbs Esc first; zoom survives.
    ui.overlays.push(Overlay::Help);
    handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r);
    assert!(ui.zoomed, "Esc closes the overlay before unzooming");
    assert!(ui.overlays.is_empty());

    handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r);
    assert!(!ui.zoomed, "Esc at the base unzooms");

    handle_key(key(KeyCode::Char('z')), &mut ui, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Char('z')), &mut ui, &mut s, &mut h, &mut r);
    assert!(!ui.zoomed, "z toggles back off");
}

#[test]
fn hint_bar_tracks_focus_and_overlays() {
    // FR-2: the bottom row shows the focused pane's keys, swaps to
    // overlay keys while one is open, and always offers `? help`.
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(rich_snapshot());
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();

    let bottom_row = |ui: &UiState, w: u16, h: u16| -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    ui,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    None,
                    100_000,
                )
            })
            .unwrap();
        row_text(&terminal, h - 1)
    };

    let alerts = bottom_row(&UiState::new(), 120, 40);
    assert!(alerts.contains("? help"), "got: {alerts}");
    assert!(
        alerts.contains("ack all non-warnings"),
        "alerts focus shows alert keys, got: {alerts}"
    );

    let mut hints_ui = UiState::new();
    hints_ui.focus = FocusTarget::Hints;
    let hints = bottom_row(&hints_ui, 120, 40);
    assert!(
        hints.contains("P pin top hint"),
        "hints focus shows hint keys, got: {hints}"
    );
    assert!(
        !hints.contains("ack all non-warnings"),
        "alert keys must leave when focus moves, got: {hints}"
    );

    let mut overlay_ui = UiState::new();
    overlay_ui.overlays.push(Overlay::Help);
    let with_overlay = bottom_row(&overlay_ui, 120, 40);
    assert!(
        with_overlay.contains("Esc close"),
        "overlay keys replace pane keys, got: {with_overlay}"
    );
    assert!(
        !with_overlay.contains("ack all"),
        "pane keys hidden under an overlay, got: {with_overlay}"
    );

    // FR-2.3: at width 40 the hints truncate with an ellipsis but
    // `? help` survives.
    let narrow = bottom_row(&UiState::new(), 40, 12);
    assert!(narrow.contains("? help"), "got: {narrow:?}");
    assert!(narrow.contains('…'), "truncation marked, got: {narrow:?}");

    // T11: the palette gets its own hint line; `q close` would be
    // a lie there since q types into the query.
    let mut palette_ui = UiState::new();
    palette_ui.overlays.push(Overlay::Palette {
        query: String::new(),
        selected: 0,
    });
    let with_palette = bottom_row(&palette_ui, 120, 40);
    assert!(
        with_palette.contains("Enter run"),
        "palette hints shown, got: {with_palette}"
    );
    assert!(
        !with_palette.contains("q close"),
        "generic overlay hints would mislead in the palette, got: {with_palette}"
    );
}

#[test]
fn enter_opens_detail_for_the_selected_item_and_esc_returns() {
    // FR-5.1/T6: Enter drills into the focused pane's selection;
    // Esc lands back on the unchanged base surface.
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    s.ingest(two_warning_snapshot());
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();

    handle_key(key(KeyCode::Char('j')), &mut ui, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
    match ui.overlays.top() {
        Some(Overlay::Detail { title, lines }) => {
            assert_eq!(title, "second-alert", "detail shows the selected item");
            assert!(
                lines.iter().any(|l| l.contains("w-second")),
                "detail body carries the fingerprint"
            );
        }
        other => panic!("expected a Detail overlay, got {other:?}"),
    }
    handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r);
    assert!(ui.overlays.is_empty(), "Esc returns to the base surface");

    // Hints pane: Enter opens the hint's detail.
    handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
    match ui.overlays.top() {
        Some(Overlay::Detail { title, .. }) => {
            assert_eq!(title, "skill://refactor", "hint detail titled by URI");
        }
        other => panic!("expected a hint Detail overlay, got {other:?}"),
    }
}

#[test]
fn enter_on_an_empty_list_is_a_noop() {
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new(); // no snapshot at all
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();
    handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
    assert!(
        ui.overlays.is_empty(),
        "Enter with nothing selected must not open a blank popup"
    );
}

#[test]
fn colon_opens_the_palette_and_typed_keys_edit_the_query() {
    // T11: inside the palette, `q` and `?` are text, not globals.
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    s.ingest(rich_snapshot());
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();

    handle_key(key(KeyCode::Char(':')), &mut ui, &mut s, &mut h, &mut r);
    assert!(matches!(ui.overlays.top(), Some(Overlay::Palette { .. })));

    let outcome = handle_key(key(KeyCode::Char('q')), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(
        outcome,
        KeyOutcome::Redraw,
        "q must not quit in the palette"
    );
    handle_key(key(KeyCode::Char('?')), &mut ui, &mut s, &mut h, &mut r);
    match ui.overlays.top() {
        Some(Overlay::Palette { query, .. }) => {
            assert_eq!(query, "q?", "typed characters land in the query")
        }
        other => panic!("palette must stay open, got {other:?}"),
    }

    handle_key(key(KeyCode::Backspace), &mut ui, &mut s, &mut h, &mut r);
    match ui.overlays.top() {
        Some(Overlay::Palette { query, .. }) => assert_eq!(query, "q"),
        other => panic!("expected palette, got {other:?}"),
    }

    handle_key(key(KeyCode::Esc), &mut ui, &mut s, &mut h, &mut r);
    assert!(ui.overlays.is_empty(), "Esc closes the palette");
}

#[test]
fn palette_enter_replays_the_selected_command() {
    // T11/TR-006: executing 'zoom pane' must behave exactly like
    // pressing `z`, because the palette replays the key.
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    s.ingest(rich_snapshot());
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();

    handle_key(key(KeyCode::Char(':')), &mut ui, &mut s, &mut h, &mut r);
    for c in "zoom".chars() {
        handle_key(key(KeyCode::Char(c)), &mut ui, &mut s, &mut h, &mut r);
    }
    handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
    assert!(ui.zoomed, "palette 'zoom pane' must zoom");
    assert!(ui.overlays.is_empty(), "palette closes after running");

    // 'quit' from the palette quits, exactly like q at the base.
    handle_key(key(KeyCode::Char(':')), &mut ui, &mut s, &mut h, &mut r);
    for c in "quit".chars() {
        handle_key(key(KeyCode::Char(c)), &mut ui, &mut s, &mut h, &mut r);
    }
    let outcome = handle_key(key(KeyCode::Enter), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(outcome, KeyOutcome::Quit);

    // A query matching nothing keeps the palette open on Enter.
    let mut ui2 = UiState::new();
    handle_key(key(KeyCode::Char(':')), &mut ui2, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Char('x')), &mut ui2, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Char('x')), &mut ui2, &mut s, &mut h, &mut r);
    handle_key(key(KeyCode::Enter), &mut ui2, &mut s, &mut h, &mut r);
    assert!(
        matches!(ui2.overlays.top(), Some(Overlay::Palette { .. })),
        "Enter with no match keeps the palette for query fixes"
    );
}

#[test]
fn palette_renders_query_and_filtered_commands() {
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(rich_snapshot());
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();
    let mut ui = UiState::new();
    ui.overlays.push(Overlay::Palette {
        query: "filter".into(),
        selected: 1,
    });

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|f| {
            draw(
                f,
                &ui,
                &snap_state,
                &hint_state,
                &research_state,
                None,
                100_000,
            )
        })
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("Commands"), "palette frame visible: {text}");
    assert!(text.contains("filter hints: token"), "matches listed");
    assert!(
        !text.contains("zoom pane"),
        "non-matching commands filtered out"
    );
}

#[test]
fn question_mark_toggles_the_help_overlay() {
    // FR-3.1: `?` opens help; `?` again (with help topmost) closes
    // it. The rendered overlay carries the help title.
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    s.ingest(rich_snapshot());
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();

    handle_key(key(KeyCode::Char('?')), &mut ui, &mut s, &mut h, &mut r);
    assert!(
        matches!(ui.overlays.top(), Some(Overlay::Help)),
        "? must open the help overlay"
    );

    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|f| draw(f, &ui, &s, &hint_state, &research_state, None, 100_000))
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("Help"), "help overlay must render");
    assert!(
        text.contains("next pane"),
        "help must list the keymap table's actions"
    );

    handle_key(key(KeyCode::Char('?')), &mut ui, &mut s, &mut h, &mut r);
    assert!(ui.overlays.is_empty(), "? must close an open help overlay");
}

#[test]
fn draw_renders_topmost_overlay_over_panes() {
    // FR-4.3: an open overlay is visible on the composed frame.
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(rich_snapshot());
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();
    let mut ui = UiState::new();
    ui.overlays.push(Overlay::Detail {
        title: "OVERLAY-TITLE".into(),
        lines: vec!["overlay-body".into()],
    });
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|f| {
            draw(
                f,
                &ui,
                &snap_state,
                &hint_state,
                &research_state,
                None,
                100_000,
            )
        })
        .unwrap();
    let text = buffer_text(&terminal);
    assert!(text.contains("OVERLAY-TITLE"), "overlay must draw on top");
}

#[test]
fn ctrl_c_quits_but_plain_c_does_not() {
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(
        handle_key(ctrl_c, &mut ui, &mut s, &mut h, &mut r),
        KeyOutcome::Quit
    );
    assert_eq!(
        handle_key(key(KeyCode::Char('c')), &mut ui, &mut s, &mut h, &mut r),
        KeyOutcome::Redraw
    );
}

#[test]
fn tab_cycles_focus_and_backtab_reverses() {
    let mut ui = UiState::new();
    let mut s = ColdWindowState::new();
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();
    assert_eq!(ui.focus, FocusTarget::Alerts, "default focus is alerts");
    assert_eq!(
        handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r),
        KeyOutcome::Redraw
    );
    assert_eq!(ui.focus, FocusTarget::Hints);
    handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.focus, FocusTarget::Research);
    handle_key(key(KeyCode::Tab), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.focus, FocusTarget::Alerts, "Tab wraps around");
    handle_key(key(KeyCode::BackTab), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(ui.focus, FocusTarget::Research, "BackTab reverses");
}

#[test]
fn focused_pane_marker_follows_focus() {
    // FR-1.3: the `>` title marker must sit on exactly the focused
    // pane, and move when focus moves: legible without color.
    let mut snap_state = ColdWindowState::new();
    snap_state.ingest(rich_snapshot());
    let hint_state = HintPaneState::new();
    let research_state = ResearchPaneState::default();

    let render_with = |focus: FocusTarget| -> String {
        let ui = UiState {
            focus,
            ..UiState::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|f| {
                draw(
                    f,
                    &ui,
                    &snap_state,
                    &hint_state,
                    &research_state,
                    None,
                    100_000,
                )
            })
            .unwrap();
        buffer_text(&terminal)
    };

    let alerts_focused = render_with(FocusTarget::Alerts);
    assert!(
        alerts_focused.contains("> Alerts"),
        "alerts focused: marker must prefix the alerts title"
    );
    assert!(
        !alerts_focused.contains("> Hints"),
        "alerts focused: hints must not carry the marker"
    );

    let hints_focused = render_with(FocusTarget::Hints);
    assert!(
        hints_focused.contains("> Hints"),
        "hints focused: marker must prefix the hints title"
    );
    assert!(
        !hints_focused.contains("> Alerts"),
        "hints focused: alerts must not carry the marker"
    );
}

#[test]
fn master_ack_key_clears_non_warning_alerts() {
    // 'A' must reach the alert pane and clear caution/advisory.
    let mut s = ColdWindowState::new();
    s.ingest(Arc::new(WindowSnapshot {
        version: 1,
        timestamp_ms: 1,
        token_ledger: TokenLedger::default(),
        alerts: vec![Alert {
            fingerprint: "c1".into(),
            severity: Severity::Caution,
            title: "t".into(),
            message: "m".into(),
            band: None,
            fired_at_ms: 1,
            dwell_ticks: 1,
        }],
        hints: vec![],
        research_findings: vec![],
        plugin_health: vec![],
        load_sample: LoadSample::default(),
        next_tick_ms: 2_000,
    }));
    assert_eq!(s.visible_alerts().len(), 1);
    let mut ui = UiState::new();
    let mut h = HintPaneState::new();
    let mut r = ResearchPaneState::default();
    let outcome = handle_key(key(KeyCode::Char('A')), &mut ui, &mut s, &mut h, &mut r);
    assert_eq!(outcome, KeyOutcome::Redraw);
    assert!(
        s.visible_alerts().is_empty(),
        "master-ack 'A' did not reach the alert pane"
    );
}
