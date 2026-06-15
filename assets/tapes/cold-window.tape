# Skrills cold-window TUI Demo
# A live, governed real-time skills analysis surface: 4-tier alert policy,
# ranked hints, research dispatcher, command palette, graceful shutdown.
# Interaction order mirrors scripts/dogfood-tui.sh (the PTY contract).
# Run from the repo root: vhs assets/tapes/cold-window.tape
# (Env PATH below is absolute, matching the sibling tapes; adjust the
# repo and ttyd directories for your machine before recording.)

Output assets/gifs/cold-window.gif

Set FontSize 13
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 22ms
Set Shell "bash"

# Absolute PATH: VHS launches ttyd via Go, which refuses relative PATH
# entries, so the binary and ttyd dirs must be absolute here.
Env PATH "/home/alext/skrills/target/release:/home/alext/.local/bin:/usr/local/bin:/usr/bin:/bin"
Env RUST_LOG "off"
Env HOME "/tmp/skrills-cw-demo"

# --- hidden setup: a configured HOME so the first-run setup prompt never
#     blocks first paint (first-run flow is dogfooded separately). ---
Hide
Type "rm -rf /tmp/skrills-cw-demo && mkdir -p /tmp/skrills-cw-demo/.codex"
Enter
Type `printf '[mcp_servers.skrills]\ncommand = "skrills"\n' > /tmp/skrills-cw-demo/.codex/config.toml`
Enter
Type "clear"
Enter
Show

# --- title + narrated intro ---
# Typed comments are the only captions VHS can render, and only at a shell
# prompt. The in-TUI captions (during the live surface) are burned in after
# recording by assets/tapes/cold-window-overlay.sh -- rerun it on reshoot.
Type "# skrills cold-window --tui: a live, governed skills analysis surface"
Enter
Sleep 900ms
Type "# Watch the 4-tier policy step Advisory -> Caution -> Warning,"
Enter
Sleep 800ms
Type "# hints rank themselves, and research stream in -- all live."
Enter
Sleep 1100ms
Type "clear"
Enter
Sleep 400ms

# --- launch the live TUI ---
# Default 100K ceiling so every tier is reachable: the demo ledger adds
# ~8K tokens per tick, so the meter steps Advisory (20K) -> Caution (50K)
# -> Warning (80K) -> kill-switch (100K) over the hold below. A slow,
# deliberate 500ms base cadence (--no-adaptive pins it) makes each
# real-time tick a discrete, trackable step.
Type "skrills cold-window --tui --skill-dir assets/demo-skills --tick-rate-ms 500 --no-adaptive --alert-budget 100000 --no-bell"
Sleep 800ms
Enter

# Hold on the live surface long enough to *watch it work*: panes paint,
# the Hints pane fills in one ranked row per tick, and the token meter
# climbs one visible step per tick. First the Advisory alert lights up
# the Alerts pane (cyan), as the ledger crosses the 20K inflection.
Sleep 4000ms

# --- research is pull-only: open it so findings stream in live ---
# R expands the Research panel; with a couple of findings already fetched,
# the rest trickle in (HN, Lobsters, GitHub, Paper, TRIZ) as the hold runs.
Type "R"
Sleep 4000ms

# Keep watching: the meter crosses 50K and the alert escalates to Caution
# (yellow), then crosses 80K into a Warning (red) token-budget breach.
Sleep 4200ms

# Past 100K the hard kill-switch engages -- still a Warning, now with the
# system refusing mutating operations. The full 4-tier story on one screen.
Sleep 3200ms

# --- discoverability: the help overlay shows the full keymap ---
Type "?"
Sleep 4200ms
Type "?"
Sleep 600ms

# --- the command palette: the novice-to-expert bridge (k9s pattern) ---
Type ":"
Sleep 4200ms
Escape
Sleep 600ms

# --- the focus model: Tab cycles Alerts -> Hints -> Research ---
# Hold on each focus state so the moving highlight is legible.
Tab
Sleep 1500ms
Tab
Sleep 1500ms
Tab
Sleep 1300ms

# --- drill into the fired alert for its governance detail ---
Enter
Sleep 4200ms
Escape
Sleep 700ms

# --- graceful shutdown within the 2-second budget ---
Ctrl+C
Sleep 2200ms
