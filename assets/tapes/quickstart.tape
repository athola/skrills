# Skrills Quickstart Demo
# Cross-CLI skill sync for Claude Code, Codex, Copilot, and Cursor
# Run: vhs assets/tapes/quickstart.tape

Output assets/gifs/quickstart.gif

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

# Title
Type "# Skrills: Unified Skills across Claude, Codex, Copilot & Cursor"
Enter
Sleep 800ms
Type "clear"
Enter
Sleep 400ms

# Setup demo environment with skills in Claude directory
Type "# Set up demo environment with sample skills"
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
Type "# Skills in Claude Code directory"
Enter
Sleep 500ms
Type "ls ~/.claude/skills"
Enter
Sleep 2s

# Validate skills across all four CLIs
Type "# Validate skills for Claude, Codex, Copilot, and Cursor"
Enter
Sleep 500ms
Type "skrills validate --target both --autofix"
Enter
Sleep 3s

# Analyze skills for token usage
Type "# Analyze token usage and get optimization suggestions"
Enter
Sleep 500ms
Type "skrills analyze --suggestions"
Enter
Sleep 4s

# Show metrics
Type "# View aggregate metrics about discovered skills"
Enter
Sleep 500ms
Type "skrills metrics"
Enter
Sleep 4s

# Sync skills to all CLIs
Type "# Sync skills from Claude Code to all other CLIs"
Enter
Sleep 500ms
Type "skrills sync-all"
Enter
Sleep 3s

# Verify sync
Type "# Verify skills synced to Codex and Copilot"
Enter
Sleep 500ms
Type "find ~/.codex/skills ~/.copilot/skills -name 'SKILL.md' 2>/dev/null | head -5"
Enter
Sleep 2s

# Final message
Type "echo '# Skills now work across all four platforms!'"
Enter
Sleep 2s
