# Skrills MCP Demo
# Demonstrates MCP server integration with Claude Code
# Run from skrills root: vhs assets/tapes/mcp.tape

Output assets/gifs/mcp.gif

Set FontSize 13
Set Width 1200
Set Height 700
Set Theme "Catppuccin Mocha"
Set Padding 20
Set TypingSpeed 50ms
Set Shell "bash"

# Set up environment - must be in skrills project directory
Env HOME "/home/alext"
Env PATH "/home/alext/.nvm/versions/node/v25.2.1/bin:/home/alext/.local/bin:/home/alext/skrills/target/release:/usr/local/bin:/usr/bin:/bin"
Env SKRILLS_SKILL_DIR "/home/alext/skrills/assets/demo-skills"

# Ensure we're in the skrills project directory (required for -s project scope)
Type "cd /home/alext/skrills"
Enter
Sleep 500ms

# Title
Type "# Skrills MCP Server: Skills via Model Context Protocol"
Enter
Sleep 800ms
Type "clear"
Enter
Sleep 400ms

# First, demonstrate actual functionality with the CLI
Type "# First, let's see what the MCP tools can do:"
Enter
Sleep 500ms

# Validate skills
Type "skrills validate"
Enter
Sleep 3s

# Show metrics
Type "# Aggregate skill metrics:"
Enter
Sleep 300ms
Type "skrills metrics"
Enter
Sleep 4s

# Now set up MCP
Type "clear"
Enter
Sleep 300ms
Type "# Now let's expose this functionality to Claude Code via MCP:"
Enter
Sleep 1s

# Show MCP serve command
Type "skrills serve --help | head -8"
Enter
Sleep 3s

# Clean up any existing config before adding
Type "# Add skrills MCP server to Claude Code:"
Enter
Sleep 500ms
Type "rm -f .mcp.json 2>/dev/null; claude mcp add skrills -s project -e SKRILLS_SKILL_DIR=$SKRILLS_SKILL_DIR -- $(which skrills) serve"
Enter
Sleep 8s

# Check MCP server status
Type "# Verify MCP server is connected:"
Enter
Sleep 300ms
Type "claude mcp list 2>&1 | tail -5"
Enter
Sleep 12s

# List available MCP tools
Type "# List available MCP tools (22 tools):"
Enter
Sleep 500ms
Type "skrills serve --list-tools | head -14"
Enter
Sleep 4s

# Demonstrate Claude calling skrills
Type "# Let's ask Claude to validate our skills:"
Enter
Sleep 500ms
Type "claude -p 'Execute skrills validate and quote the exact output' --dangerously-skip-permissions --max-turns 3 2>&1"
Enter
Sleep 45s

# Final message
Type "# Done! Claude Code can invoke skrills tools via MCP or CLI."
Enter
Sleep 2s

