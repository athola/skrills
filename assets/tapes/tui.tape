# Skrills Dashboard TUI Demo
# Interactive skill dashboard with panels for Skills, Activity, Metrics, MCP
# Run: vhs assets/tapes/tui.tape

Output assets/gifs/dashboard.gif

Set FontSize 13
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 50ms
Set Shell "bash"

# Set up environment - use demo skills from repo
Env PATH "/Users/alex.thola/skrills/target/release:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"
Env SKRILLS_SKILL_DIR "/Users/alex.thola/skrills/assets/demo-skills"
Env SKRILLS_INCLUDE_CLAUDE "1"
Env HOME "/tmp/skrills-demo"

# Prepare demo skills
Type "mkdir -p ~/.claude/skills && cp -r $SKRILLS_SKILL_DIR/* ~/.claude/skills/"
Enter
Sleep 300ms
Type `echo '{"mcpServers":{"skrills":{}}}' > ~/.claude/.mcp.json`
Enter
Sleep 300ms

# First run sync to populate data (auto-accept defaults)
Type "yes | skrills tui 2>/dev/null || true"
Enter
Sleep 3s

# Launch the dashboard TUI
Type "skrills dashboard --skill-dir ~/.claude/skills"
Enter
Sleep 3s

# Navigate through skills list
Type "j"
Sleep 500ms
Type "j"
Sleep 500ms
Type "j"
Sleep 500ms

# Switch to Activity panel
Type@100ms "	"
Sleep 2s

# Switch to Metrics panel
Type@100ms "	"
Sleep 2s

# Back to Skills panel
Type@100ms "	"
Sleep 1s

# Toggle sort order
Type "s"
Sleep 2s

# Quit
Type "q"
Sleep 1s
