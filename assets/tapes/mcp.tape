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
Type "# Skrills MCP Server: Testing with Claude Code"
Enter
Sleep 800ms
Type "clear"
Enter
Sleep 400ms

# Show skrills MCP serve command
Type "# Skrills provides an MCP server for Claude Code"
Enter
Sleep 500ms
Type "skrills serve --help | head -15"
Enter
Sleep 3s

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
Sleep 3s

# Test MCP tool inside Claude Code CLI
Type "# Test the autoload-snippet MCP tool inside Claude Code"
Enter
Sleep 500ms
Type "claude -p 'Use mcp__skrills__autoload-snippet to load skrills://skill/coding-assistant' --model haiku --max-budget-usd 0.05 2>&1 | head -30"
Enter
Sleep 15s

# Show what tools are available
Type "# Skrills MCP provides these tools to Claude Code:"
Enter
Sleep 500ms
Type "echo 'mcp__skrills__autoload-snippet - Load skill content'"
Enter
Sleep 400ms
Type "echo 'mcp__skrills__search-skills - Search by name/content'"
Enter
Sleep 400ms
Type "echo 'mcp__skrills__resolve-dependencies - Find dependencies'"
Enter
Sleep 400ms
Type "echo 'mcp__skrills__recommend-skills - Get recommendations'"
Enter
Sleep 2s

# Final message
Type "# MCP tools work seamlessly inside Claude Code!"
Enter
Sleep 2s
