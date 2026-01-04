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
Env PATH "/home/alext/.local/bin:/home/alext/skrills/target/release:/usr/local/bin:/usr/bin:/bin"

# Title
Type "# Skrills: Unified Skills for Claude Code & Codex"
Enter
Sleep 800ms
Type "clear"
Enter
Sleep 400ms

# Show available skills count
Type "# First, let's see what skills we have available"
Enter
Sleep 500ms
Type "find ~/.claude/plugins/cache -name 'SKILL.md' | wc -l"
Enter
Sleep 2s

# Validate skills
Type "# Validate skills work on both Claude Code and Codex"
Enter
Sleep 500ms
Type "skrills validate --errors-only"
Enter
Sleep 5s

# Analyze skills for token usage
Type "# Analyze skills for token optimization opportunities"
Enter
Sleep 500ms
Type "skrills analyze --suggestions --min-tokens 300"
Enter
Sleep 6s

# Show sync status
Type "# Check sync status between Claude Code and Codex"
Enter
Sleep 500ms
Type "skrills sync-status"
Enter
Sleep 4s

# Sync skills to Codex
Type "# Sync skills from Claude Code to Codex"
Enter
Sleep 500ms
Type "skrills sync"
Enter
Sleep 4s

# Show metrics
Type "# View metrics about discovered skills"
Enter
Sleep 500ms
Type "skrills metrics"
Enter
Sleep 4s

# Final message
Type echo "# Skills now work seamlessly on both platforms!"
Enter
Sleep 2s
