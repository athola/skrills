# Skrills Quickstart Demo
# Bidirectional skill sync for Claude Code ↔ Codex
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
Env PATH "/home/alext/.local/bin:/home/alext/skrills/target/release:/usr/local/bin:/usr/bin:/bin"
Env SKRILLS_SKILL_DIR "/home/alext/skrills/assets/demo-skills"
Env SKRILLS_INCLUDE_CLAUDE "1"
Env HOME "/tmp/skrills-demo"

# Title
Type "# Skrills: Unified Skills for Claude Code & Codex"
Enter
Sleep 800ms
Type "clear"
Enter
Sleep 400ms

# Setup demo environment with skills in Claude directory
Type "# Set up demo environment with sample skills"
Enter
Sleep 300ms
Type "mkdir -p ~/.claude/skills ~/.codex/skills"
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

# Validate skills
Type "# Validate skills for Claude Code and Codex compatibility"
Enter
Sleep 500ms
Type "skrills validate"
Enter
Sleep 3s

# Analyze skills for token usage
Type "# Analyze skills for token usage and quality"
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

# Show sync status
Type "# Check sync status between Claude Code and Codex"
Enter
Sleep 500ms
Type "skrills sync-status"
Enter
Sleep 3s

# Sync skills to Codex
Type "# Sync skills from Claude Code to Codex"
Enter
Sleep 500ms
Type "skrills sync"
Enter
Sleep 3s

# Verify sync
Type "# Verify skills were synced to ~/.codex/skills"
Enter
Sleep 500ms
Type "find ~/.codex/skills -name 'SKILL.md' | head -5"
Enter
Sleep 2s

# Final message
Type "echo '# Skills now work seamlessly on both platforms!'"
Enter
Sleep 2s
