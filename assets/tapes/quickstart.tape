# Skrills Quickstart Demo
# Cross-CLI skill sync for Claude Code, Codex, Copilot, and Cursor
# Run: vhs assets/tapes/quickstart.tape

Output assets/gifs/quickstart.gif

Set FontSize 13
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 25ms
Set Shell "bash"

# Set up environment - use demo skills from repo.
# Absolute paths: VHS launches ttyd via Go, which refuses relative PATH
# entries, so the binary and ttyd dirs must be absolute. Adjust the repo
# and ttyd directories for your machine before recording.
Env PATH "/home/alext/skrills/target/release:/home/alext/.local/bin:/usr/local/bin:/usr/bin:/bin"
Env SKRILLS_SKILL_DIR "/home/alext/skrills/assets/demo-skills"
Env SKRILLS_INCLUDE_CLAUDE "1"
Env HOME "/tmp/skrills-demo"
# Quiet the tracing logs so command output stays clean on screen.
Env RUST_LOG "off"

# Title
Type "# Skrills: one set of skills, four AI CLIs -- Claude, Codex, Copilot, Cursor"
Enter
Sleep 800ms
Type "clear"
Enter
Sleep 400ms

# Setup demo environment with skills in Claude directory
Type "# Step 1: seed sample skills into Claude Code"
Enter
Sleep 300ms
Type "mkdir -p ~/.claude/skills ~/.codex/skills ~/.copilot/skills"
Enter
Sleep 300ms
Type "cp -r $SKRILLS_SKILL_DIR/* ~/.claude/skills/"
Enter
Sleep 300ms
# Mark skrills as configured to skip setup prompt
Type `echo '{"mcpServers":{"skrills":{}}}' > ~/.claude/.mcp.json`
Enter
Sleep 500ms

# Show available skills
Type "# Step 1 done: the skills now live in Claude Code"
Enter
Sleep 500ms
Type "ls ~/.claude/skills"
Enter
Sleep 3500ms

# Validate skills across all four CLIs
Type "# Step 2: validate them against all four CLIs (--autofix repairs)"
Enter
Sleep 500ms
Type "skrills validate --target both --autofix"
Enter
Sleep 5s

# Analyze skills for token usage
Type "# Step 3: analyze token cost, with reduction suggestions"
Enter
Sleep 500ms
Type "skrills analyze --suggestions"
Enter
Sleep 6s

# Show metrics
Type "# Step 4: review aggregate metrics across every skill"
Enter
Sleep 500ms
Type "skrills metrics"
Enter
Sleep 6s

# Sync skills to all CLIs
Type "# Step 5: sync from Claude Code out to every other CLI"
Enter
Sleep 500ms
Type "skrills sync-all"
Enter
Sleep 5s

# Verify sync
Type "# Step 5 done: the same skills landed in Codex and Copilot"
Enter
Sleep 500ms
Type "find ~/.codex/skills ~/.copilot/skills -name 'SKILL.md' 2>/dev/null | head -5"
Enter
Sleep 3500ms

# Final message
Type "echo '# Skills now work across all four platforms!'"
Enter
Sleep 3500ms
