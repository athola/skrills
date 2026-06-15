---
maturity: growing
type: lessons
updated: 2026-06-10
---

# Lessons Learned

Insights, failed approaches, rework, and blockers, captured blamelessly
so the team replicates what worked and avoids what did not.

## Active index

| ID | Status | Title | Date |
|----|--------|-------|------|
| LL-001 | open | Status bar content was invisible in the live TUI (Borders::TOP at height 1) | 2026-06-10 |
| LL-002 | open | Spec FR-7.1 asserted 'repaints only on events' without reading the event loop | 2026-06-10 |

## Lessons

## LL-001: Status bar content was invisible in the live TUI (Borders::TOP at height 1)

- Status: open
- Date: 2026-06-10
- Phase: execute
- Category: testing
- Owner: -
- Links: -
<!-- key: b4e6a54469f6 -->

### What happened

While wiring the contextual hint bar (T7), a buffer-level probe showed the 1-row status bar rendered only ' Status ───' - the Borders::TOP block consumed the single row, hiding the tick/token/alert content the bar exists to show.

### What did not work

Existing tests asserted via StatusBar::render_to_string (string building) and never inspected the rendered buffer at the live 1-row height, so the bug shipped invisible to the suite.

### Root cause

Tests validated the data path, not the pixel path; the only buffer-level render test used a 3-row area where the border plus content both fit.

### Recommendation / action item

Status bar now renders borderless content directly (committed in Sprint 2); new tests assert against the actual bottom row of the composed frame. Pattern to keep: every 'renders X' claim needs at least one buffer assertion at the widget's real production geometry.

## LL-002: Spec FR-7.1 asserted 'repaints only on events' without reading the event loop

- Status: open
- Date: 2026-06-10
- Phase: execute
- Category: process
- Owner: -
- Links: -
<!-- key: 7765799fd7db -->

### What happened

The specification claimed event-driven-only repaints were 'already true'; the event loop has had a deliberate 250ms repaint floor (status-bar quota/clock freshness) since the cold-window TUI landed.

### What did not work

Writing an invariant into the spec from the mission brief's assumption instead of from the code.

### Root cause

Specify phase summarized module doc comments and skipped the run loop body where the tokio interval lives.

### Recommendation / action item

The repaint-floor invariant was corrected against the code (ratatui buffer diffing makes idle-floor repaints zero-write) and now lives in book/src/cold-window.md under "Design model and research basis". Rule going forward: every 'already true' claim gets a code citation (file:line) before it is written down.

## Archive

Superseded or deprecated entries sink here; nothing is deleted (git keeps history).

<!-- ENTRY TEMPLATE -- copy a block into the Lessons section above the Archive
heading, assign the next LL-NNN id, and fill it in. The journal_append helper
does this automatically; this block is the fallback for hand-editing.

## LL-NNN: <short lesson title>

- Status: open
- Date: YYYY-MM-DD
- Phase: execute | review
- Category: process | technology | requirements | testing | communication
- Owner: <who carries the follow-up>
- Links: <PR/commit/issue>, <related TR-NNN>

### What happened

<blameless, factual: the situation/activity>

### What went well / where we got lucky

<successes worth replicating>

### What did not work

<the gap or failure>

### Root cause

<5 Whys / contributing factors>

### Recommendation / action item

- Action: <specific change> -- Owner: <name> -- Due: <date> -- Status: <...>
-->
