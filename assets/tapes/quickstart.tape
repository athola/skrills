# Skrills Quickstart Demo
# Bidirectional skill sync for Claude Code ↔ Codex
# Run: vhs assets/tapes/quickstart.tape

Output assets/gifs/quickstart.gif

Set FontSize 13
Set Width 1000
Set Height 600
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 40ms
Set Shell "bash"
Env PATH "/home/alext/.local/bin:/home/alext/skrills/target/release:/usr/local/bin:/usr/bin:/bin"

# Title
Type "# Skrills: Unified Skills for Claude Code & Codex"
Enter
Sleep 500ms
Type "clear"
Enter
Sleep 300ms

# Show CLI overview
Type "skrills --help"
Enter
Sleep 4s

# Check sync status between Claude Code and Codex
Type "# Check what's different between Claude Code and Codex"
Enter
Sleep 500ms
Type "skrills sync-status"
Enter
Sleep 4s

# Validate skills for both targets
Type "# Validate skills work on both platforms"
Enter
Sleep 500ms
Type "skrills validate --errors-only"
Enter
Sleep 4s

# Analyze token usage
Type "# Analyze skills for token optimization"
Enter
Sleep 500ms
Type "skrills analyze --suggestions --min-tokens 500"
Enter
Sleep 5s

# Sync everything
Type "# Sync all configurations bidirectionally"
Enter
Sleep 500ms
Type "skrills sync-all --dry-run"
Enter
Sleep 4s

# Final message
Type "# Skills now work seamlessly on both Claude Code and Codex!"
Enter
Sleep 2s
