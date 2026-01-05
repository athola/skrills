# Skrills MCP Demo
# Demonstrates MCP server integration with Claude Code
# Run: vhs assets/tapes/mcp.tape

Output assets/gifs/mcp.gif

Set FontSize 13
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 50ms
Set Shell "bash"

# Set up environment
Env PATH "/home/alext/.nvm/versions/node/v25.2.1/bin:/home/alext/.local/bin:/home/alext/skrills/target/release:/usr/local/bin:/usr/bin:/bin"
Env SKRILLS_SKILL_DIR "/home/alext/skrills/assets/demo-skills"

# Title
Type "# Skrills MCP Server: Skills via Model Context Protocol"
Enter
Sleep 800ms
Type "clear"
Enter
Sleep 400ms

# Show skrills MCP serve command
Type "# Skrills provides an MCP server for Claude Code"
Enter
Sleep 500ms
Type "skrills serve --help | head -12"
Enter
Sleep 6s

# Show MCP configuration format
Type "# Add to ~/.claude/settings.local.json mcpServers section:"
Enter
Sleep 500ms
Type "# skrills: { command: skrills, args: [serve] }"
Enter
Sleep 2s

# Check MCP server status
Type "# Verify skrills MCP server is connected in Claude Code"
Enter
Sleep 500ms
Type "claude mcp list | grep skrills"
Enter
Sleep 10s

# Show available MCP tools
Type "# Available MCP tools:"
Enter
Sleep 500ms
Type "echo 'create-skill, search-skills-github, sync-from-claude, sync-skills'"
Enter
Sleep 1s
Type "echo 'sync-commands, sync-mcp-servers, sync-preferences, sync-all'"
Enter
Sleep 1s
Type "echo 'validate-skills, analyze-skills, skill-metrics, recommend-skills'"
Enter
Sleep 2s

# Show skill discovery
Type "# See discovered skills via CLI"
Enter
Sleep 500ms
Type "skrills metrics"
Enter
Sleep 6s

# Final message
Type "# Skills work seamlessly with Claude Code via MCP!"
Enter
Sleep 2s
