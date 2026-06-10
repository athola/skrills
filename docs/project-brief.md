# Project Brief: Minimalist TUI Interface for skrills

Mission: tui-interface-0.8.x | Phase: brainstorm (complete) | Date: 2026-06-09
Research basis: docs/research/2026-06-09-minimalist-tui-interface-design.md

## Problem

skrills has two ratatui surfaces (cold-window dashboard, server TUI) that grew
feature-by-feature. There is no deliberate interface model governing what is visible by
default, how users discover keys, how data-rich detail is reached, or how the layout
behaves on narrow terminals (including SSH from a phone). Each new feature risks adding
permanent chrome.

## Goal

Adopt a single, research-backed interaction model - the "lazygit/gitui model" - as the
governing design for all skrills TUI surfaces:

> Minimal fixed default surface; all depth behind toggleable overlays, drill-downs, and
> a popup stack; discoverability via a contextual hint line; layout collapses gracefully
> by width.

## Design Decisions (from research)

1. **Default surface stays as-is, codified**: the existing cold-window panes (alert,
   hint, research, status bar) are the complete default surface. New features MUST NOT
   add permanent panes; they ship as overlays or drill-downs.
2. **Contextual command bar** (gitui `cmdbar.rs` pattern): the bottom line shows only
   keys valid for the focused pane; `?` opens a scoped help overlay. Replaces any static
   keybinding text.
3. **Popup stack** (gitui `popup_stack.rs` + `tui-popup` pattern): configuration and
   data-rich views open as modal popups pushed onto a stack (Esc pops). Depth never costs
   permanent screen space.
4. **Select-to-reveal drill-down** (atuin inspector pattern): Enter on a list item opens
   a detail view; Tab/number keys cycle pane focus; `z` (or `+`) zooms the focused pane
   fullscreen.
5. **Responsive tiers, formalized**: keep the existing narrow/medium/wide branching but
   codify breakpoints (~<80 cols: stacked single-column; <50 cols: compact mode showing
   only the focused pane + status line, atuin `style=auto` analog). `Constraint::Min`/
   `Fill` so panes degrade before clipping; minimum-size guard screen below hard floor.
6. **Mobile-usable = narrow + single-key**: no separate mobile build. Mobile viability
   comes from the compact tier, zoom-pane, and ensuring every action is reachable via
   single keys or the palette (no CTRL/ALT-only bindings, glass keyboards lack them).
7. **Command palette (`:`)** as novice-expert bridge (k9s pattern) - optional, last
   priority.
8. **Restraint rules**: semantic color only (meaning survives without color); repaint on
   state change, never on timer (already true); no animation/fade effects; do not break
   terminal text selection where avoidable.

## Non-Goals

- No framework change (stay on ratatui 0.30 + crossterm; no Textual/Bubble Tea rewrite).
- No mouse-first interaction; mouse stays optional.
- No new permanent panes or tabs in this mission.
- No web/GUI surface changes (browser cold-window untouched except shared state).

## Success Criteria

- All cold-window features reachable from the minimal default surface via documented keys.
- Hint bar shows context-correct keys for every focusable pane; `?` overlay exists.
- Layout renders usefully (no clipping/panics) at 40x12, 80x24, and 200x50; TestBackend
  tests cover each tier.
- Config/detail views open as popups; Esc always returns to the base surface.
- Compact tier usable over SSH from a phone-width terminal (~45 cols).

## Risks

- Branch is already RED-zone (scope-guard waived by user directive for this mission).
- Server TUI (crates/server/src/tui.rs) and cold-window TUI may drift; mitigation: shared
  helpers where practical, identical key vocabulary.

## Validated Approaches Considered

- Textual-style whitespace/web idioms: rejected (density-first community signal, Rust
  stack, rewrite cost).
- Formal breakpoint framework: rejected as over-engineering; ecosystem convention is
  width-conditional branching, which skrills already does.
- Separate mobile binary/mode: rejected; narrow-tier + zoom + single-key coverage achieves
  the same with zero new surface.

## Next Phase

specify - convert decisions 2-6 into testable requirements with acceptance criteria
(Skill(attune:project-specification)).
