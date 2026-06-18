---
maturity: growing
type: tradeoffs
updated: 2026-06-10
---

# Tradeoffs

Decisions made over this project's lifetime, and the alternatives
we deliberately gave up. Records the *why*, not just the *what*.

## Active index

| ID | Status | Title | Date |
|----|--------|-------|------|
| TR-001 | accepted | Esc pops overlays instead of quitting the cold-window TUI | 2026-06-10 |
| TR-002 | accepted | Hybrid focus model: focus governs hints/Enter/zoom, disjoint pane keys preserved | 2026-06-10 |
| TR-003 | accepted | Compact tier hides unfocused panes instead of squeezing all panes | 2026-06-10 |
| TR-004 | accepted | Overlay stack implemented in-tree; no tui-popup dependency | 2026-06-10 |
| TR-005 | accepted | Add minimal per-pane selection cursors to enable Enter drill-down | 2026-06-10 |
| TR-006 | accepted | Command palette executes by replaying key codes through handle_key | 2026-06-10 |

## Decisions

## TR-001: Esc pops overlays instead of quitting the cold-window TUI

- Status: accepted
- Date: 2026-06-10
- Phase: specify
- Deciders: -
- Links: ['book/src/cold-window.md (keybindings)', 'crates/dashboard/src/cold_window/keymap.rs']
<!-- key: 967f84e27b4f -->

### Context & problem

Baseline binds q, Esc, and Ctrl-C all to quit. The overlay-stack model (FR-4) needs a universal back/dismiss key, and Esc is the ecosystem convention (lazygit, gitui, k9s).

### Options considered

| Option | Pros | Cons / what it sacrifices |
|--------|------|---------------------------|
| Esc pops topmost overlay; no-op at base; q/Ctrl-C quit (chosen) | matches lazygit/gitui muscle memory; safe (no accidental quit) | breaking change for users who quit via Esc |
| Keep Esc=quit, use Backspace or x to dismiss overlays | no breaking change | violates every surveyed TUI convention; hint bar must teach a nonstandard key |

### Decision

Esc dismisses the topmost overlay and does nothing at the base surface; q and Ctrl-C remain quit. Documented as breaking in CHANGELOG.

### Y-statement

In the context of adopting an overlay stack, facing the need for a dismiss key, we chose Esc-pops over Esc-quits to match TUI convention, accepting a one-time muscle-memory break.

## TR-002: Hybrid focus model: focus governs hints/Enter/zoom, disjoint pane keys preserved

- Status: accepted
- Date: 2026-06-10
- Phase: specify
- Deciders: -
- Links: ['crates/dashboard/src/cold_window/focus.rs']
<!-- key: 1af567ea9322 -->

### Context & problem

Baseline has no focus model; pane keymaps are disjoint (A/d, 0-5/P, R) and every key is forwarded to all panes. A contextual hint bar and Enter/zoom need a focus target.

### Options considered

| Option | Pros | Cons / what it sacrifices |
|--------|------|---------------------------|
| Hybrid: add focus for hints/Enter/zoom, keep disjoint global pane keys (chosen) | zero regression for existing keys; smallest change; focus only where semantics need it | two routing concepts coexist |
| Full focus routing: keys only reach the focused pane | single uniform model (gitui style) | breaks every existing binding from unfocused panes; larger rewrite |

### Decision

Focus is added for hint-bar content, Enter drill-down, and zoom targeting only; existing disjoint keymaps keep working globally (FR-1.4).

### Y-statement

In the context of adding discoverability to a focus-less TUI, we chose a hybrid focus model over full focus routing to preserve existing muscle memory, accepting two coexisting routing concepts.

## TR-003: Compact tier hides unfocused panes instead of squeezing all panes

- Status: accepted
- Date: 2026-06-10
- Phase: specify
- Deciders: -
- Links: ['book/src/cold-window.md (Design model and research basis)', 'crates/dashboard/src/cold_window/tui']
<!-- key: 24f66be2e7ed -->

### Context & problem

Below 45 columns (phone SSH), three stacked panes leave each a few unreadable lines. Research guidance (Kim/Moritz/Hullman 2021; Elmqvist/Fekete 2009): aggregate or relocate detail, never shrink the same content.

### Options considered

| Option | Pros | Cons / what it sacrifices |
|--------|------|---------------------------|
| Compact tier renders only the focused pane + status bar; Tab switches visibility (chosen) | each pane fully readable; aligns with atuin auto-compact and research guidance | no at-a-glance view of all panes on phones |
| Keep stacking all panes at any width | everything nominally visible | 3-line slivers are unreadable; clipping artifacts |

### Decision

New LayoutMode::Compact (<45 cols) shows focused pane + status only; hard floor guard below 20x6 (FR-6).

### Y-statement

In the context of phone-width terminals, we chose focus-equals-visibility over squeezed stacking so the visible pane stays usable, accepting loss of simultaneous overview.

## TR-004: Overlay stack implemented in-tree; no tui-popup dependency

- Status: accepted
- Date: 2026-06-10
- Phase: specify
- Deciders: -
- Links: ['crates/dashboard/src/cold_window/overlay.rs']
<!-- key: 45576312e0f3 -->

### Context & problem

FR-4 needs a modal overlay stack. ratatui ecosystem offers tui-popup/tui-widgets; gitui implements its own popup_stack.rs in-tree.

### Options considered

| Option | Pros | Cons / what it sacrifices |
|--------|------|---------------------------|
| In-tree Vec-backed overlay stack using ratatui Clear + centered Rect (chosen) | no new dependency; full control over key routing; pattern is ~100 lines; testable with TestBackend | small amount of code ratatui ecosystem already wrote |
| Add tui-popup dependency | maintained widget; draggable popups | new supply-chain surface for a trivial pattern; key routing still needs custom code |

### Decision

Implement the overlay stack in-tree following the gitui popup_stack pattern (NFR-2).

### Y-statement

In the context of adding modal overlays, we chose an in-tree stack over the tui-popup crate to avoid a dependency for a 100-line pattern, accepting minor reimplementation.

## TR-005: Add minimal per-pane selection cursors to enable Enter drill-down

- Status: accepted
- Date: 2026-06-10
- Phase: plan
- Deciders: -
- Links: ['crates/dashboard/src/cold_window/state.rs']
<!-- key: be0ec74d0131 -->

### Context & problem

No pane has a selection model today; Enter-to-drill (FR-5.1) needs a target. Alternative was drilling into a fixed 'latest/top' item with no cursor.

### Options considered

| Option | Pros | Cons / what it sacrifices |
|--------|------|---------------------------|
| Minimal Up/Down/j/k selection index per pane, active when focused (chosen) | drill-down targets any item; standard TUI idiom; small clamped-index state | +state in three panes; research pane selection semantics fuzzy |
| Selection-free drill-down into newest/top item only | zero new state | cannot inspect older alerts/hints; violates details-on-demand for most data |

### Decision

Add clamped selection indices (T5) rendered with a non-color marker; research pane may degrade to entry-level selection.

### Y-statement

In the context of drill-down, we chose minimal selection cursors over a selection-free model so any item is inspectable, accepting new pane state.

## TR-006: Command palette executes by replaying key codes through handle_key

- Status: accepted
- Date: 2026-06-10
- Phase: execute
- Deciders: -
- Links: ['crates/dashboard/src/cold_window/keymap.rs palette_commands']
<!-- key: fe3dff49a731 -->

### Context & problem

The ':' palette (FR-8, deferred item picked up by user directive) needs to execute commands. Alternatives: a dedicated command enum with its own execution functions, or replaying the equivalent key code through the existing routing.

### Options considered

| Option | Pros | Cons / what it sacrifices |
|--------|------|---------------------------|
| Replay the entry key code through handle_key after closing the palette (chosen) | single source of execution semantics; palette can never drift from keybindings; sync test enforces every entry maps to an owned key; ~60 lines total | commands are limited to what keys can express; one level of controlled recursion |
| Dedicated PaletteCommand enum with explicit execution arms | commands could take arguments later | duplicates every action; help/hint/palette can disagree; more code to keep in sync |

### Decision

Palette entries carry a label plus the KeyCode they replay; Enter pops the palette and re-enters handle_key with a synthetic key event (TR-006).

### Y-statement

In the context of palette execution, we chose key replay over a command enum so the palette and keybindings share one behavior, accepting key-expressible commands only.

## Archive

Superseded or deprecated entries sink here; nothing is deleted (git keeps history).

<!-- ENTRY TEMPLATE -- copy a block into the Decisions section above the
Archive heading, assign the next TR-NNN id, and fill it in. The journal_append
helper does this automatically; this block is the fallback for hand-editing.

## TR-NNN: <short decision title>

- Status: proposed
- Date: YYYY-MM-DD
- Phase: brainstorm | specify | plan | execute | review
- Deciders: <names/roles>
- Links: <PR/commit/issue>, <code paths>

### Context & problem

<the situation forcing a choice>

### Decision drivers

- <competing quality / constraint>

### Options considered

| Option | Pros | Cons / what it sacrifices |
|--------|------|---------------------------|
| A (chosen) | ... | ... |
| B | ... | ... |

### Decision

We chose A.

### Y-statement

In the context of <X>, facing <concern>, we chose A over B,
to achieve <quality>, accepting <the sacrifice / road not taken>.

### Consequences

- Positive: <what gets easier>
- Negative / debt accepted: <what gets harder; revisit trigger>
-->
