# Specification: Minimalist TUI Interface (cold-window)

Mission: tui-interface-0.8.x | Phase: specify | Date: 2026-06-09
Brief: docs/project-brief.md | Research: docs/research/2026-06-09-minimalist-tui-interface-design.md

## Scope

Applies to the cold-window TUI (`crates/dashboard/src/cold_window/`). The server TUI
(`crates/server/src/tui.rs`) adopts the same key vocabulary but is otherwise out of
scope. The browser cold-window surface is untouched.

## Current State (baseline)

- Three width tiers: Wide (>=80), Medium (60-79), Narrow (<60) via `layout_mode`.
- Four panes (alerts, hints, research, status bar) rendered as pure functions; disjoint
  keymaps (`A`/`d`, `0`-`5`/`P`, `R`), no focus model.
- `q`/`Esc`/`Ctrl-C` all quit. No help overlay, no contextual key hints, no popups.

## Functional Requirements

### FR-1: Focus model

- FR-1.1: Exactly one of {alerts, hints, research} holds focus at any time; default
  focus is alerts.
- FR-1.2: `Tab` cycles focus forward (alerts -> hints -> research -> alerts);
  `Shift-Tab`/`BackTab` cycles backward. `1`/`2`/`3` jump directly.
- FR-1.3: The focused pane is visually distinct (border style/title emphasis) using
  semantic styling that also reads without color (e.g. bold title or marker character).
- FR-1.4: Existing pane keys keep working regardless of focus (disjoint keymaps are
  preserved); focus governs the hint bar content, Enter, and zoom targets only.

Acceptance: TestBackend test asserts default focus, Tab/BackTab cycle order, direct
jump keys, and that the focused pane's border differs from unfocused panes in both
styled and color-stripped buffers.

### FR-2: Contextual hint bar

- FR-2.1: The bottom status line gains a right-aligned (or trailing) hint segment
  showing only keys valid for the focused pane plus globals (`?` help, `q` quit).
- FR-2.2: Hint content changes when focus changes and when an overlay opens (overlay
  keys replace pane keys while the overlay is topmost).
- FR-2.3: When the terminal is too narrow to fit all hints, hints truncate from the
  right with an ellipsis rather than wrapping or clipping mid-cell; the `?` hint is
  always retained.

Acceptance: TestBackend tests assert hint text per focused pane, per topmost overlay,
and truncation behavior at width 40.

### FR-3: Help overlay (`?`)

- FR-3.1: `?` toggles a help overlay listing all keybindings grouped by pane/global,
  rendered as a centered popup over a `Clear`-ed region.
- FR-3.2: The overlay is scoped: the focused pane's section is listed first.
- FR-3.3: `?`, `Esc`, or `q` closes the overlay (overlay consumes keys; underlying
  panes receive none while it is open).

Acceptance: TestBackend test opens help, asserts focused-pane section ordering, asserts
an alert-pane key (`d`) does not reach AlertPane while help is open, closes via Esc.

### FR-4: Overlay stack

- FR-4.1: A single `Vec`-backed overlay stack owns all modal surfaces (help overlay,
  future config/detail popups). Topmost overlay receives all keys.
- FR-4.2: `Esc` pops the topmost overlay; when the stack is empty, `Esc` does nothing
  (BREAKING: Esc no longer quits; `q`/`Ctrl-C` remain quit). Documented in CHANGELOG.
- FR-4.3: Overlays render last (over panes) via `Clear` + centered rect sized
  proportionally to the frame with sane minimums; overlays never exceed the frame.
- FR-4.4: Render order and key routing are pure functions testable without a terminal.

Acceptance: unit tests for push/pop routing, Esc-pop vs Esc-noop-when-empty, q-quits
with overlay open closes overlay first (lazygit convention: q pops, quits only at base)
OR quits directly - decision recorded in tradeoffs; render test asserts overlay cells
overwrite pane cells.

### FR-5: Drill-down and zoom

- FR-5.1: `Enter` on the focused pane opens a detail overlay for the current selection
  (alerts: full alert detail; hints: full hint text + metadata; research: full research
  entry). Reuses the FR-4 stack.
- FR-5.2: `z` toggles zoom: the focused pane occupies the full body (status line stays);
  zoom state survives focus changes; `z` again or `Esc` restores the normal layout.
- FR-5.3: Zoom is layout-level (a `LayoutMode`-independent override in `plan_layout`),
  not a per-pane reimplementation.

Acceptance: TestBackend tests: Enter pushes a detail overlay containing the selected
item's text; zoomed layout gives the focused pane the full body rect at all three
tiers; Esc/z restores prior rects.

### FR-6: Compact tier (mobile)

- FR-6.1: New `LayoutMode::Compact` when `width < 45`: only the focused pane plus the
  one-line status bar are rendered; other panes are hidden (not squeezed).
- FR-6.2: Tab/1/2/3 switch which pane is visible (focus == visibility in Compact).
- FR-6.3: Below a hard floor (`width < 20 || height < 6`) render a one-line "terminal
  too small (need >= 20x6)" guard instead of panes.
- FR-6.4: All interface actions remain reachable with single keys or Tab/Enter/Esc -
  no CTRL/ALT-required bindings for any pane or overlay action (Ctrl-C quit is a
  redundant escape hatch, not the only path).

Acceptance: layout tests at 40x12 (Compact: focused pane + status only), 44 vs 45 width
boundary, 19x6 and 20x5 guard cases; keymap audit test asserts no registered binding
requires CONTROL/ALT modifiers.

### FR-7: Restraint invariants (regression guards)

- FR-7.1 (amended during execution): Repaints occur on snapshot/key/resize events plus
  the pre-existing, deliberate 250ms repaint floor that keeps the status bar's
  quota/clock fresh (the original "already true" claim was wrong - the floor predates
  this mission and is load-bearing). Acceptable because ratatui diffs buffers: an idle
  floor repaint with unchanged content emits zero terminal writes, so screen readers
  and scrollback are not spammed. No new timer-driven render paths may be added.
- FR-7.2: Pane content styling uses semantic colors with meaning duplicated in text
  (severity tags remain textual). No animation, fade, or spinner in the base surface.

## Non-Functional Requirements

- NFR-1: All new logic (layout planning, focus, overlay stack, hint bar content) is
  pure and TestBackend/unit testable; the crossterm event loop gains no new logic
  beyond routing.
- NFR-2: No new dependencies beyond ratatui 0.30 / existing workspace crates;
  `tui-popup` may be vendored as a pattern but not added as a dependency unless it
  removes more code than it adds.
- NFR-3: No regression in existing cold-window tests; `make format && make lint &&
  make test` pass.
- NFR-4: Key vocabulary documented in book/src/cold-window.md and shared with the
  server TUI where keys overlap (q quit, ? help if implemented there later).

## Out of Scope

- Command palette (`:`) - deferred to backlog (brief decision 7, last priority).
- Server TUI implementation changes beyond doc alignment.
- Mouse interaction, browser surface, new permanent panes.

## Requirement -> Test Map

| Req | Test location (planned) |
|-----|------------------------|
| FR-1 | cold_window/tui.rs tests: focus_cycle, focus_visual_distinct |
| FR-2 | status_bar.rs / tui.rs tests: hint_bar_per_focus, hint_truncation_w40 |
| FR-3 | overlay tests: help_overlay_scoped, help_consumes_keys |
| FR-4 | overlay stack unit tests: push_pop_routing, esc_semantics |
| FR-5 | drilldown_detail_overlay, zoom_layout_all_tiers |
| FR-6 | layout tests: compact_tier_boundaries, too_small_guard, no_modifier_keys_audit |
| FR-7 | doc/test: event_driven_repaint_only, semantic_color_textual_severity |
