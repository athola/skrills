# Claude Code Demo Script: Hook-Based Integration

This script outlines a demonstration of the hook-based `skrills` integration for Claude Code. This integration uses semantic trigram matching to automatically inject relevant skills into your prompts, via the `prompt.on_user_prompt_submit` hook.

## Prerequisites

- Install Claude Code CLI.
- Ensure your Claude Code environment is clean, meaning the `~/.claude/` directory should be in a default state.
- Clone this repository locally.

## Terminal Preparation

```bash
# Navigate to the skrills repository
cd /path/to/skrills

# Install skrills with Claude hook integration
./scripts/install.sh --client claude

# Verify installation
ls -la ~/.claude/hooks/
ls -la ~/.claude/mcp_servers.json
```

Confirm that the output lists the `prompt.on_user_prompt_submit` hook file and indicates the successful registration of the `skrills` MCP server.

## Demo Script

Start Claude Code from within the repository:

```bash
cd /path/to/skrills
claude
```

### 1. Verify Hook Integration

**Prompt:** "What hooks are currently active in this Claude Code session?"

**Expected behavior:** Claude should confirm that the `skrills` hook is active within the session.

### 2. List Available Skills

**Prompt:** "List all available skills from the skrills MCP server."

**Expected behavior:** Claude should use the `list-skills` tool, displaying all `SKILL.md` files from `~/.codex/skills/`.

### 3. Demonstrate Semantic Matching

**Prompt:** "I need help with test-driven development for a new feature."

**Expected behavior:**
- The hook intercepts the prompt and identifies keywords like "test-driven" and "development".
- The system should automatically inject the `test-driven-development.md` skill.
- Claude should then provide TDD guidance, based on the injected skill.

**Follow-up Prompt:** "Show me what skills were autoloaded for my previous prompt."

**Expected behavior:** Claude should use the `autoload-snippet` tool to detail which skills were matched and injected into the prompt.

### 4. Test Keyword-Based Loading

**Prompt:** "Help me debug a flaky integration test."

**Expected behavior:**
- The system detects keywords: "debug", "flaky", "test".
- It automatically injects skills such as `systematic-debugging.md` and `condition-based-waiting.md`.
- Claude should offer a debugging workflow tailored to addressing race conditions.

### 5. Verify Context-Aware Skills

**Prompt:** "I want to brainstorm different approaches for a caching layer."

**Expected behavior:**
- The system identifies keywords: "brainstorm", "approaches".
- It injects the `brainstorming.md` skill.
- Claude should then engage in the brainstorming skill's Socratic method, guiding the discussion.

### 6. Show MCP Server Status

**Prompt:** "What's the status of the skrills MCP server?"

**Expected behavior:** Claude should display the MCP server's status via the `runtime-status` tool, detailing the skills directory, the number of loaded skills, current cache status, and relevant configuration settings.

### 7. Demonstrate Skill Manifest

**Prompt:** "Show me the manifest of available skills with their trigger keywords."

**Expected behavior:** Claude should use MCP tools to show the skill metadata, which includes skill names, descriptions, and their associated trigger keywords.

### 8. Test Skill Pinning

**Prompt:** "How do I pin the code review skill so it's always included?"

**Expected behavior:** Claude should explain the process of pinning skills using `set-runtime-options`.

### 9. Refresh Skills Cache

**Prompt:** "I just added a new skill file. Refresh the skills cache."

**Expected behavior:** Claude should demonstrate the `refresh-cache` tool, which reloads skill metadata without requiring a server restart.

### 10. Show Token Efficiency

**Prompt:** "How does semantic matching reduce token usage?"

**Expected behavior:** Claude should explain how `skrills` reduces token usage by injecting only the most relevant skills, filtering them based on trigram similarity scores, and provide a comparative example of token consumption (all available skills versus only the matched skills).

### 11. Claude-Specific Skill Sync

**Prompt:** "Show me how to sync Claude Code skills into the skrills directory."

**Expected behavior:** Claude should demonstrate the `sync-from-claude` tool. This tool mirrors skills from `~/.claude/skills` to `~/.codex/skills-mirror`, allowing other clients to use them.

## Verification Checklist

- [ ] Hook is active: `cat ~/.claude/hooks/prompt.on_user_prompt_submit`
- [ ] MCP server is registered: `grep skrills ~/.claude/mcp_servers.json`
- [ ] Skills auto-inject on relevant prompts.
- [ ] MCP tools are accessible (`list-skills`, `runtime-status`).
- [ ] Semantic matching works as expected.
- [ ] Cache refresh works without a restart.

## Recording a GIF

```bash
# Record the session
asciinema rec demo-claude-hooks.cast

# Convert to GIF
npx agg demo-claude-hooks.cast demo-claude-hooks.gif \
  --theme dracula \
  --font 'JetBrainsMono Nerd Font' \
  --speed 1.1 \
  --cols 100 \
  --rows 30
```

## Demo Flow Tips

- To ensure adequate vertical spacing, press Enter twice after each Claude response.
- For concise responses, append `(brief)` to your prompts.
- When requesting skill lists, specify `show the first 5 only` to keep the output short.
- Conclude the demonstration by asking: `Show token usage for this session.`

## Key Differentiators (Hook-Based)

1. **Automatic Injection**: Skills are automatically loaded and injected without requiring explicit MCP tool calls.
2. **Zero-Touch Workflow**: As you type prompts, relevant skills are automatically injected into the context.
3. **Context-Aware**: The integration hook intercepts and analyzes the complete user prompt before it's submitted.
4. **Claude-Native**: This integration is native, directly using Claude Code's hook system.
5. **Session-Persistent**: The integration hook remains active for the entire Claude Code session.

## Example Output Flow

```
You: I need help with test-driven development

[The integration hook intercepts your prompt, identifies a semantic match for "test-driven" and "development", and then automatically injects the `test-driven-development.md` skill into the context.]

Claude: I can help you with test-driven development...
[Provides TDD-specific guidance]

You: What skills were loaded?

Claude: For your previous prompt, I automatically loaded:
- test-driven-development.md (match score: 0.89)
```
