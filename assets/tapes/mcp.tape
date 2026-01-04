# Skrills MCP Demo
# Demonstrates MCP server usage with both Claude Code and Codex
# Run: vhs assets/tapes/mcp.tape

Output assets/gifs/mcp.gif

Set FontSize 13
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 50ms
Set Shell "bash"
Env PATH "/home/alext/.local/bin:/home/alext/skrills/target/release:/usr/local/bin:/usr/bin:/bin"

# Title
Type "# Skrills MCP Server: Claude Code & Codex Integration"
Enter
Sleep 800ms
Type "clear"
Enter
Sleep 400ms

# Show help for serve command
Type "# The skrills MCP server works with both Claude Code and Codex"
Enter
Sleep 500ms
Type "skrills serve --help"
Enter
Sleep 3s

# Check MCP configuration
Type "# Check if skrills is configured in Claude Code"
Enter
Sleep 500ms
Type "cat ~/.claude/settings.json | grep -A 5 'mcpServers' | head -20"
Enter
Sleep 2s

# Show doctor command
Type "# Diagnose MCP configuration for Codex"
Enter
Sleep 500ms
Type "skrills doctor"
Enter
Sleep 4s

# Sync MCP servers
Type "# Sync MCP server configurations between platforms"
Enter
Sleep 500ms
Type "skrills sync-mcp-servers --dry-run"
Enter
Sleep 4s

# Show available MCP tools
Type "# List MCP tools provided by skrills"
Enter
Sleep 500ms
Type "echo 'Skrills MCP provides: autoload-snippet, search-skills, resolve-dependencies, recommend-skills'"
Enter
Sleep 2s

# Final message
Type echo "# Use skrills serve in MCP config for both platforms!"
Enter
Sleep 2s
