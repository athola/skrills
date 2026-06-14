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

# --- title ---
Type "# skrills cold-window --tui: a live, governed skills analysis surface"
Enter
Sleep 1200ms
Type "clear"
Enter
Sleep 400ms

# --- launch the live TUI ---
# Low budget so the 4-tier policy fires a WARNING on screen; fast tick so
# the real-time meter visibly climbs within the recording window.
Type "skrills cold-window --tui --skill-dir assets/demo-skills --tick-rate-ms 150 --alert-budget 2000 --no-bell"
Sleep 800ms
Enter
# First paint, then a few ticks: panes appear, token meter climbs past the
# 2.0K ceiling, and the WARNING alert lights up the Alerts pane.
Sleep 3500ms

# --- discoverability: the help overlay shows the full keymap ---
Type "?"
Sleep 2600ms
Type "?"
Sleep 600ms

# --- the command palette: the novice-to-expert bridge (k9s pattern) ---
Type ":"
Sleep 2400ms
Escape
Sleep 600ms

# --- the focus model: Tab cycles Alerts -> Hints -> Research ---
Tab
Sleep 900ms
Tab
Sleep 900ms
Tab
Sleep 700ms

# --- drill into the fired alert for its governance detail ---
Enter
Sleep 2400ms
Escape
Sleep 700ms

# --- graceful shutdown within the 2-second budget ---
Ctrl+C
Sleep 1200ms
