# Implementation Plan: Minimalist TUI Interface (cold-window)

Mission: tui-interface-0.8.x | Phase: plan | Date: 2026-06-09
Spec: docs/specification.md | Brief: docs/project-brief.md

## Architecture

All new logic follows the existing cold-window pattern: pure, TestBackend-testable
functions and plain state structs; the crossterm `run` loop in `tui.rs` only routes.

New components (all in `crates/dashboard/src/cold_window/`):

| Component | File | Responsibility |
|-----------|------|----------------|
| `FocusTarget` enum + cycle fns | `focus.rs` (new) | Which pane holds focus; Tab/BackTab/1-3 transitions (FR-1) |
| `Overlay` enum + `OverlayStack` | `overlay.rs` (new) | Vec-backed modal stack; key routing; centered-rect render-over with Clear (FR-4) |
| Help overlay content | `overlay.rs` | Keybinding listing grouped by pane, focused-first (FR-3) |
| Detail overlay content | `overlay.rs` | Render selected alert/hint/research item full-text (FR-5.1) |
| Selection cursors | `state.rs` + pane files | Up/Down/j/k selection within focused pane (FR-5 prerequisite) |
| Hint segment | `status_bar.rs` | Trailing contextual key hints, truncate-with-ellipsis (FR-2) |
| `LayoutMode::Compact` + zoom + guard | `tui.rs` | <45-col focused-pane-only tier; `z` zoom override; 20x6 floor guard (FR-5.2, FR-6) |

Key routing order in `handle_key` (replaces current flat routing):
1. `Ctrl-C`/`q` quit (q closes topmost overlay first, quits at base - lazygit convention,
   per TR-001 discussion; Esc pops, no-op at base).
2. If overlay stack non-empty: topmost overlay consumes the key. Stop.
3. Global: `Tab`/`BackTab`/`1`-`3` focus, `?` help, `z` zoom, `Enter` drill-down,
   `Up/Down/j/k` selection in focused pane.
4. Existing disjoint pane keys forwarded to all panes (unchanged, FR-1.4).

Note: `1`-`3` for focus jump conflicts with HintPane's existing `0`-`5` keys. Resolution
task T2 audits HintPane bindings and remaps focus-jump OR hint keys; acceptance requires
zero ambiguous bindings (current candidate: keep `0`-`5` for hints since they predate
this work, drop direct focus-jump keys, keep Tab/BackTab only).

## File Structure

| File | Action | Tasks |
|------|--------|-------|
| `crates/dashboard/src/cold_window/focus.rs` | create | T1 |
| `crates/dashboard/src/cold_window/overlay.rs` | create | T3, T4, T6 |
| `crates/dashboard/src/cold_window/state.rs` | edit | T5 |
| `crates/dashboard/src/cold_window/alert_pane.rs` | edit | T1, T5 |
| `crates/dashboard/src/cold_window/hint_pane.rs` | edit | T1, T2, T5 |
| `crates/dashboard/src/cold_window/research_pane.rs` | edit | T1, T5 |
| `crates/dashboard/src/cold_window/status_bar.rs` | edit | T7 |
| `crates/dashboard/src/cold_window/tui.rs` | edit | T1-T9 (routing, layout) |
| `crates/dashboard/src/cold_window/mod.rs` | edit | T1, T3 (exports) |
| `book/src/cold-window.md` | edit | T10 |
| `docs/CHANGELOG.md` | edit | T10 |

## Tasks (TDD: every task starts with failing tests)

**T1. Focus model** (FR-1) - 3 pts
`FocusTarget` enum {Alerts, Hints, Research}; default Alerts; Tab/BackTab cycling;
focused pane border emphasis (bold title + marker, readable without color).
Tests: focus_cycle_order, default_focus, focused_border_distinct (styled + raw text).
Depends: none.

**T2. Key vocabulary audit** (FR-1.2, FR-6.4) - 2 pts
Inventory all bindings across panes + globals into one table-driven test; assert
disjointness and no CONTROL/ALT-required actions; settle focus-jump vs hint `0`-`5`
conflict. Tests: no_ambiguous_bindings, no_modifier_required_audit.
Depends: T1.

**T3. Overlay stack** (FR-4) - 5 pts
`Overlay` enum + `OverlayStack(Vec<Overlay>)`; push/pop; topmost consumes keys; Esc
pops / no-op at base; q pops then quits at base; centered-rect sizing (proportional,
clamped to frame); render-over with `Clear` after panes.
Tests: push_pop_routing, esc_pop_then_noop, q_pops_then_quits, overlay_cells_overwrite,
overlay_never_exceeds_frame (1x1 frame edge case).
Depends: T1 (focus-aware content later, stack itself independent).
BREAKING: Esc no longer quits at base - CHANGELOG entry in T10.

**T4. Help overlay** (FR-3) - 3 pts
`?` toggles; content generated from the T2 binding table (single source of truth);
focused pane section first; consumes all keys while open.
Tests: help_scoped_ordering, help_consumes_pane_keys, help_toggle_close_esc.
Depends: T2, T3.

**T5. Selection cursors** (FR-5 prerequisite) - 3 pts
Per-pane selection index (alerts list, hints list, research entries) moved with
Up/Down/j/k when focused; clamped to list bounds; rendered as highlighted row
(semantic: also a marker char). Tests: selection_clamps, selection_only_when_focused,
selection_marker_visible_without_color.
Depends: T1.

**T6. Drill-down detail overlay** (FR-5.1) - 3 pts
Enter on focused pane pushes detail overlay rendering the selected item's full content
(alert detail / hint text + metadata / research entry). Tests:
enter_pushes_detail_with_selected_text per pane, esc_returns_to_base.
Depends: T3, T5.

**T7. Contextual hint bar** (FR-2) - 3 pts
Status bar trailing segment: focused pane's keys + `? help` + `q quit`; overlay keys
replace pane keys when stack non-empty; right-truncate with ellipsis keeping `?`.
Content derived from the T2 binding table. Tests: hints_per_focus, hints_under_overlay,
hint_truncation_w40_keeps_help.
Depends: T2, T3.

**T8. Zoom** (FR-5.2, FR-5.3) - 2 pts
`z` toggles zoom flag; `plan_layout` override gives focused pane the full body at every
tier; Esc also unzooms when stack empty (before its no-op). Tests:
zoom_full_body_all_tiers, zoom_survives_focus_change, esc_unzooms_then_noop.
Depends: T1.

**T9. Compact tier + size guard** (FR-6) - 3 pts
`LayoutMode::Compact` (width < 45): focused pane + status only; focus == visibility;
hard floor guard rendering one-line message when width < 20 || height < 6. Tests:
compact_tier_boundary_44_45, compact_focused_only, guard_19x6_20x5,
existing draw_survives_tiny_and_huge_areas still green.
Depends: T1.

**T10. Restraint guards, docs, integration** (FR-7, NFR-3, NFR-4) - 3 pts
Event-driven-repaint assertion (no timer render path); semantic-color/textual-severity
test; update book/src/cold-window.md key tables; CHANGELOG breaking-change entry (Esc);
server TUI doc alignment (shared vocabulary note); full `make format && make lint &&
make test`. Depends: all.

## Dependency Graph (acyclic)

```
T1 -> T2 -> T4, T7
T1 -> T5 -> T6
T1 -> T8, T9
T3 -> T4, T6, T7
T1 -> T3 (weak: content only)
all -> T10
```

## Critical Path

T1 -> T3 -> T6 -> T10 (focus -> overlay stack -> drill-down -> integration), ~14 pts of
the 30-pt total.

## Sprints

- Sprint 1 (foundation, 10 pts): T1, T2, T3 - after this the breaking Esc change and
  stack exist; everything else is additive.
- Sprint 2 (features, 14 pts): T4, T5, T6, T7, T8.
- Sprint 3 (responsive + polish, 6 pts): T9, T10.

## Risks

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Key conflicts (focus jump vs hint 0-5) | High | T2 table-driven audit before any overlay work; single binding table feeds help + hint bar |
| Esc breaking change annoys users | Medium | CHANGELOG + hint bar shows `q quit` permanently |
| tui.rs grows past maintainability (830 lines now) | Medium | New code in focus.rs/overlay.rs; tui.rs gains only routing |
| Selection semantics unclear for research pane (non-list content) | Medium | T5 may scope research selection to entry-level only; acceptable degradation |
| Branch already RED zone | Accepted | User directive; each sprint is independently commit-able and green |

## FR Coverage

FR-1: T1, T2 | FR-2: T7 | FR-3: T4 | FR-4: T3 | FR-5: T5, T6, T8 | FR-6: T9 | FR-7: T10
NFR-1: all (pure fns) | NFR-2: T3 in-tree | NFR-3: T10 | NFR-4: T10
