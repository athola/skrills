# Research Report: Minimalist TUI Interface Design for skrills

Session: 2026-06-09 | Channels: code (GitHub), discourse (HN/Lobsters/blogs), academic (OpenAlex)
Question: What simple, minimalist, elegant TUI interface should skrills employ as its main
surface while keeping data-rich features toggleable for drill-down and dynamic configuration,
with dynamic resizing and a mobile-usable narrow mode?

## Executive Summary

Three independent channels converge on the same answer: the "lazygit model" adapted to
ratatui idioms. A small, fixed set of panes as the default surface; every data-rich or
configurable feature behind a toggleable overlay, drill-down pane, or popup stack; a
one-line contextual hint bar for discoverability; width-conditional layout collapse for
narrow terminals. No mainstream TUI ships a true "mobile mode" - the practical mechanisms
are atuin-style auto-compact rendering, zoom-to-single-pane, and width-breakpoint reflow.

## Theme 1: Minimal default surface, depth on demand

The community's reference architecture is now literally a genre name ("lazygit-style"):
stacked side panels plus one main view, unchanged since day one, with Enter to drill in
and a zoom/maximize key as the accordion escape hatch.

- **gitui** (22k stars, ratatui) is the flagship implementation to copy at the code level:
  `src/popup_stack.rs` + `src/popups/` keep the base screen to three lists; every
  data-rich operation pushes a modal onto a popup stack so depth never costs permanent
  screen space. [code, 0.95]
- **atuin** (30k stars, ratatui) models select-to-reveal: a minimal list default with an
  `inspector` drill-down detail view. [code, 0.9]
- **crates-tui** (official ratatui reference app) demonstrates toggleable help, modal
  prompts, and select-to-reveal detail in idiomatic ratatui. [code, 0.8]
- Academic support: Shneiderman's mantra (overview first, zoom and filter, then
  details-on-demand) and Bach et al. 2022's analysis of 144 dashboards - `detail-on-demand`
  (47% of corpus), `drilldown` (55%), `parameterization` (52%) are the named patterns;
  information overload was the dominant failure mode reported. "Spotlight the single most
  important value first, then add." [academic, 10/10]
- Springer & Whittaker 2020 caveat: the disclosure trigger must be obvious and tied to the
  user's current focus, or detail is ignored. [academic, 7/10]

## Theme 2: Discoverability without chrome

The single most-quoted praise for lazygit on HN (436 pts, Nov 2025): "All the different
commands are all visible, so it lacks the usual TUI experience of 'what key do I need
again?'"

- **gitui `cmdbar.rs`**: a one-line contextual command bar that changes per focused
  component; `?` opens a full help overlay scoped to context. [code, 0.95]
- **zellij**: mode-aware bottom hint bar, hideable for a clean UI. [code, 0.6]
- **yazi/helix**: which-key style pending-keymap hints on chord prefixes. [code, 0.7]
- Jens Roemer's TUI design essay: "a manual is a last resort" - show hotkeys inline next
  to functions; tig's 100+ undifferentiated hotkeys are the named anti-pattern.
  [discourse, 0.9]
- Tak et al. 2013: users satisfice - shortcuts are not adopted just by existing; surface
  them at the moment of use. Cockburn et al. 2014: feedforward - co-locate command name
  and binding. [academic, 8-9/10]
- HotStrokes (CHI 2019) + novice-to-expert literature: a fuzzy-searchable command palette
  bridges discoverability and speed (k9s `:resource` is the field-proven example).
  [academic, 7/10]

## Theme 3: Density with restraint

- Jesse Duffield (Lazygit Turns 5): terminal users self-select for speed; show as much
  context as possible, but every pixel must earn it. The command log (transparency about
  what the tool actually runs) became one of lazygit's most-loved features. [discourse, 0.95]
- btop's process-list fade is the canonical "aesthetics over usability" complaint;
  Charm-style heavy padding draws skepticism from density-first users. Semantic color
  (meaning survives without color) is the stated ideal. [discourse, 0.9]
- Sarikaya et al. 2018: a monitoring dashboard's job is at-a-glance consolidation on one
  screen - justifies a dense-but-bounded default with interaction for depth. [academic, 9/10]

## Theme 4: Dynamic resizing and narrow-terminal (mobile) use

No widely-starred TUI implements formal breakpoints; the ecosystem convention is ad-hoc
`if area.width < N` branching plus `Constraint::Min`/`Fill` so panes degrade before
clipping.

- **ratatui 0.30 primitives**: `Flex` layouts, `Min`/`Max`/`Fill`/`Percentage` constraints
  (see `flex`, `constraints`, `constraint-explorer`, `popup` examples). [code, 0.9]
- **atuin `style = auto`**: switches to compact rendering when the terminal is too short;
  `inline_height` renders as a small inline panel - the model for height adaptation.
  [code, 0.9]
- **zellij/lazygit pane zoom**: one keystroke makes any pane fullscreen - the universal
  escape hatch for tiny terminals. [code, 0.85]
- **superfile**: min-terminal-size guard screen instead of corrupt rendering. [code, 0.5]
- Mobile reality (Blink/Termius/Termux over SSH + tmux): the dominant pain point is the
  keyboard layer (no free ESC/CTRL/arrows on glass), ahead of width. Single-key
  navigation and a command palette matter more than layout for phone use. No mainstream
  TUI ships a true single-column mode - a gap, not a solved pattern. [discourse, 0.65]
- Kim, Moritz & Hullman 2021 + Elmqvist & Fekete 2009: when space is tight, aggregate or
  relocate detail behind interaction rather than shrinking; the narrow layout should carry
  less simultaneous information, not the same content compressed. [academic, 8-9/10]

## Theme 5: Architecture for toggleable panes

- **ratatui/templates component template**: `Component` trait (init/handle_events/update/
  draw), action channels, config-driven keybindings - makes toggleable panes, focus
  routing, and per-component keymaps trivial. skrills' cold-window panes already follow
  the pure-render-function half of this pattern. [code, 0.75]
- **ratatui/tui-widgets**: `tui-popup` (Clear-based modal overlay) and `tui-scrollview`
  (virtual buffer + viewport) are the official widgets for overlays and small-screen
  scrolling. [code, 0.8]

## Contrarian Views

- Lobsters "Why TUIs are back" (75 comments): TUIs "combine the worst parts of CLI and
  GUI" - broken text selection, scrollback, and grid-locked layouts. Mitigation: never
  break terminal affordances; keep mouse selection working where possible.
- Accessibility critique (Lobsters, 67 pts; Hashimoto: "the whole stack is rotten"):
  reactive redraws spam screen readers. Mitigation: minimize gratuitous repaints; repaint
  on state change, not on a timer (skrills' cold-window already repaints on snapshot
  events).
- Textual/Charm philosophy (whitespace, animation, web idioms) is the respected
  counterpoint to density-first design - but the density-first camp is the one that
  praises tools as "elegant and minimal."

## Recommendation (summary)

Adopt the lazygit/gitui interaction model on the existing ratatui 0.30 + component-pane
architecture:

1. Default surface: existing panes, one focused, minimal chrome.
2. Contextual one-line hint bar (gitui cmdbar pattern) + `?` help overlay.
3. Popup stack for all data-rich/config actions (gitui popup_stack + tui-popup).
4. Select-to-reveal detail / Enter-to-drill-down (atuin inspector pattern).
5. Width-breakpoint reflow: multi-column above ~80 cols, stacked below, plus
   zoom-pane key and a compact mode (atuin `auto` pattern) for narrow/mobile terminals.
6. Optional command palette (`:`) as the novice-expert bridge.

## Bibliography

Code: gitui, atuin, lazygit, yazi, zellij, taskwarrior-tui, crates-tui, ratatui examples,
ratatui/tui-widgets, ratatui/templates, awesome-ratatui.
Discourse: HN 45878578 (lazygit, 436 pts), Jesse Duffield "Lazygit Turns 5", Lobsters
"Why TUIs are back", Lobsters "The text mode lie", Jens Roemer "TUI design", Changelog
#511 (Will McGugan), cosyra.com "TUI apps on phone", awesometui.com, rothgar/awesome-tuis.
Academic: Shneiderman 1996 (VL); Bach et al. 2022 (TVCG); Sarikaya et al. 2018 (TVCG);
Cockburn et al. 2014 (CSUR); Tak et al. 2013 (IwC); Cui et al. 2019 (CHI); Kim et al.
2021 (CGF); Elmqvist & Fekete 2009 (TVCG); Heer & Shneiderman 2012 (CACM); Springer &
Whittaker 2020 (TiiS).
