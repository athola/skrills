---
name: html-portal-generator
description: Convert a codebase into a self-contained HTML portal app for ingestion into AI application systems. Produces a single deployable HTML file with embedded CSS, JS, and data.
version: 1.0.0
---

# HTML Portal Generator

Convert any codebase into a single, self-contained HTML portal application. The output is one `.html` file with embedded styles, scripts, and data that works offline, on `file://` protocol, and can be uploaded into AI application portals.

## When to Use

- You have a codebase (CLI tool, library, API, framework) and need an interactive HTML representation of its functionality
- The HTML will be uploaded into a smart AI portal or application integration system
- You need all features to work standalone without a running server
- You want users to explore, validate, analyze, and interact with the product from a browser

## Process

Follow these phases in order. Each phase builds on the previous.

### Phase 1: Codebase Analysis

Understand what the product does before writing any HTML.

1. **Read project documentation**: README, CLAUDE.md, architecture docs, changelogs
2. **Identify core crates/packages**: Map each module's purpose and public API
3. **Catalog user-facing features**: CLI commands, API endpoints, configuration options
4. **Extract data structures**: Response types, configuration schemas, enum values
5. **Find existing UI code**: Dashboards, templates, stylesheets (reuse their design language)

Produce a mental inventory:
- Feature list (what the product does)
- Data catalog (what entities/types exist)
- Command/API reference (how users interact)
- Visual identity (colors, fonts, layout patterns from existing UI)

### Phase 2: Reference Analysis (if provided)

If the user provides a reference HTML app:

1. Read the first and last ~3KB to understand structure (large files may be minified)
2. Identify the design pattern: CSS variables, layout system, component structure
3. Note the interaction model: controls bar, status bar, grid layout, tab navigation
4. Extract the color palette and typography from CSS custom properties

### Phase 3: Architecture Design

Design the portal as a single-page app with tab navigation.

**Required structural elements:**
- Sticky header with product name, version badge, and summary stats
- Tab navigation bar for switching between views
- Main container with `max-width` for readability
- Sticky status bar footer with links to docs/repo
- Toast notification system for user feedback

**View planning - map each product feature to a view:**

| Feature Type | View Pattern |
|---|---|
| Entity browsing (skills, packages, endpoints) | Card grid with search/filter/sort + detail panel |
| Validation/checking | Split editor (textarea input) + results panel |
| Analysis/metrics | Input + stat cards + bar charts |
| Configuration/creation | Form inputs + live preview + download |
| Format conversion | Side-by-side input/output with target selector |
| Reference docs (commands, APIs) | Searchable/filterable list with copy-on-click |
| Status/compatibility | Matrix grid or table |
| Dashboard/overview | Stat cards + quick actions + recent items |

**Every view must work 100% standalone.** No external API calls required. Embed demo data. Add optional live-server connection as a bonus.

### Phase 4: Data Embedding

Convert real product data into JavaScript arrays embedded in the HTML.

1. **Scan the actual codebase** for entities (skills, commands, endpoints, types)
2. **Extract metadata** from source files (frontmatter, doc comments, type definitions)
3. **Generate realistic validation states** based on actual product rules
4. **Include failure reasons** as tooltip/detail text so users understand why things pass/fail
5. **Store as `const` arrays** in the `<script>` block - no external JSON files

For large datasets (100+ items), use a build script to extract and format the data rather than hand-writing it.

If the product has local files users might want to scan:
- Add a "Scan Local" button using the File System Access API (`showDirectoryPicker`)
- Walk directories recursively, parse file frontmatter/headers
- Replace the embedded data with live-scanned results
- Store full file content on scanned items for downstream analysis

### Phase 5: Implementation

Build the HTML file following these rules:

**Single-file architecture:**
```
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Product - Portal</title>
  <style>/* All CSS here */</style>
</head>
<body>
  <!-- All HTML here -->
  <script>/* All JS here, wrapped in IIFE */</script>
</body>
</html>
```

**CSS requirements:**
- Use CSS custom properties (`:root { --bg: ...; }`) for theming
- Support dark mode (default) and light mode via `@media (prefers-color-scheme: light)`
- Reuse the product's existing color palette if it has one
- Mobile-responsive with `@media (max-width: ...)` breakpoints
- Style scrollbars, focus states, and transitions

**JavaScript requirements:**
- Wrap everything in an IIFE: `(function() { 'use strict'; ... })();`
- No inline `onclick` handlers - use `addEventListener` exclusively
- Clipboard operations must fall back to `document.execCommand('copy')` for `file://` protocol
- Every click should produce visible feedback (toast, navigation, highlight)
- Autocomplete/datalist for name inputs that reference the embedded data
- Tab navigation via `data-view` attributes and `.active` class toggling

**Interactivity checklist - every element the user can see must do something when clicked:**
- Cards/list items: select, show detail, or navigate
- Buttons: perform action with toast feedback
- Reference items: copy to clipboard on click
- Status indicators: show tooltip with explanation on hover
- External links: open in new tab with `target="_blank" rel="noopener"`
- Quick actions: run in-browser if possible, otherwise copy CLI command + link to docs

### Phase 6: Feature Implementation Patterns

**Entity Browser (the main catalog view):**
```
- Search input (filters by name + description)
- Source/category dropdown filter
- Quality/status dropdown filter
- Sort selector (alpha, by source, by status)
- Card grid with: name, source badge, description, validation tags
- Validation tags with hover tooltips showing failure reasons
- Detail panel (sticky sidebar) with full metadata on click
- Export button (download as JSON)
- Local scan button (File System Access API)
```

**Validator:**
```
- Target selector (individual CLI or "All")
- Textarea for pasting content
- Validate button - runs client-side validation logic
- Autofix button - actually modifies content:
  - Fills missing frontmatter fields
  - Converts names to kebab-case
  - Scaffolds body sections if too short
  - Adds target-specific fields (e.g., globs for Cursor)
- Quality score panel (0-100 with visual ring)
- Download button for the validated/fixed file
- Load Sample button with realistic example content
```

**Token/Size Analyzer:**
```
- Name input with datalist autocomplete from catalog
- Load button to pull content from catalog by name
- Textarea for content
- Stat cards: tokens, words, lines, characters
- Progress bar with percentage (label outside bar for small values)
- Size category with color coding
- Optimization suggestions list
```

**Format Converter:**
```
- Source format selector
- Target format selector
- Convert button (single target)
- Convert All button (generates all formats with tab switcher)
- Download button
- Side-by-side input/output layout
```

**Creator/Builder:**
```
- Form fields for metadata (name, description, version, deps)
- Target-specific fields shown/hidden based on selection
- Body textarea with placeholder template
- Live preview panel (pre element)
- File path display showing where to save for each CLI
- Dependency graph visualization
- Download and Send-to-Validator buttons
```

**Reference views (commands, tools, APIs):**
```
- Search input
- Category filter
- Each item shows: name/command, description, usage guidance
- Click to copy, with toast confirmation
- Usage guidance paragraph below each item explaining when/how to use it
```

**Dashboard:**
```
- Stat cards row (total items, categories, etc.)
- Quick Actions panel:
  - In-browser actions (green "runs here" badge): actually execute
  - CLI-only actions (muted "CLI only" badge): copy command + link to docs
- Supporting info panels with clickable items
- Recent items grid linking to the browser view
```

### Phase 7: Quality Checks

Before delivering, verify:

1. **JS syntax**: `node -e "new Function(scriptContent)"` parses without error
2. **All tabs work**: Each `data-view` has a matching `id="view-..."` element
3. **No emdashes**: Replace all `—` with `-` (AI slop reduction)
4. **No dead clicks**: Every visible element does something on click
5. **Clipboard works on file://**: All copy operations use the `execCommand` fallback
6. **Tooltips on status indicators**: Hover explains why something passed/failed
7. **Responsive**: Views stack on mobile
8. **Open in browser**: `open filename.html` and manually test each tab

### Phase 8: Iteration

The user will test and request changes. Common requests:

- "X doesn't do anything when I click it" - Add click handler with visible feedback
- "Information is missing" - Add data fields, tooltips, or guidance text
- "It should use my actual data" - Scan real files and embed or add local scan
- "Autofix doesn't change anything" - Make it fix more issues, scaffold content
- "The percentage doesn't show" - Move labels outside narrow bars
- "Link to docs" - Add `target="_blank"` links to product documentation

## Output

A single `.html` file (typically 60-120KB) that:
- Works by double-clicking in Finder/Explorer
- Works on `file://` protocol in any browser
- Optionally connects to a live server for real-time data
- Optionally scans local files via browser directory picker
- Can be uploaded directly into AI application portals
