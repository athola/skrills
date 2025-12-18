# Claude Code Demo Script: Skill Validation and Sync

This script demonstrates the `skrills` integration for Claude Code, focusing on skill validation, analysis, and synchronization.

## Prerequisites

- Install Claude Code CLI.
- Clone the skrills repository locally.

## Terminal Preparation

```bash
# Navigate to the skrills repository
cd /path/to/skrills

# Install skrills with Claude integration
./scripts/install.sh --client claude

# Verify installation
skrills doctor
```

## Demo Script

Start Claude Code from within the repository:

```bash
cd /path/to/skrills
claude
```

### 1. Verify Installation

**Prompt:** "Verify that skrills is properly installed and configured."

**Expected behavior:** Run `skrills doctor` to confirm MCP server registration.

### 2. Validate Skills

**Prompt:** "Validate all my skills for Codex CLI compatibility."

**Expected behavior:**
- Claude uses `skrills validate --target codex`
- Shows validation results for each skill
- Highlights any missing frontmatter issues

### 3. Auto-Fix Validation Issues

**Prompt:** "Fix any skills that aren't compatible with Codex CLI."

**Expected behavior:**
- Claude uses `skrills validate --target codex --autofix --backup`
- Creates backups of modified files
- Adds missing frontmatter automatically

### 4. Analyze Token Usage

**Prompt:** "Which of my skills use the most tokens?"

**Expected behavior:**
- Claude uses `skrills analyze --min-tokens 1000`
- Lists skills sorted by token count
- Identifies optimization candidates

### 5. Get Optimization Suggestions

**Prompt:** "Give me suggestions for optimizing my large skills."

**Expected behavior:**
- Claude uses `skrills analyze --suggestions`
- Provides actionable recommendations
- Identifies skills that could be split or simplified

### 6. Preview Sync Changes

**Prompt:** "What would change if I synced my Claude skills to Codex?"

**Expected behavior:**
- Claude uses `skrills sync-status --from claude`
- Shows files that would be added/updated
- Lists configuration differences

### 7. Sync Skills

**Prompt:** "Sync all my Claude configurations to Codex."

**Expected behavior:**
- Claude uses `skrills sync-all --from claude --skip-existing-commands`
- Copies skills into `~/.codex/skills/` (Codex discovery root)
- Syncs commands, MCP servers, and preferences
- Preserves existing local commands

### 8. Launch TUI

**Prompt:** "Open the interactive sync manager."

**Expected behavior:**
- Claude runs `skrills tui`
- Shows interactive terminal UI
- Allows browsing and selecting sync operations

## Verification Checklist

- [ ] Doctor reports healthy configuration: `skrills doctor`
- [ ] Validation runs successfully: `skrills validate --target both`
- [ ] Analysis provides useful insights: `skrills analyze --suggestions`
- [ ] Sync preview works: `skrills sync-status --from claude`
- [ ] Full sync completes: `skrills sync-all --from claude`

## Recording a GIF

```bash
# Record the session
asciinema rec demo-skrills.cast

# Convert to GIF
npx agg demo-skrills.cast demo-skrills.gif \
  --theme dracula \
  --font 'JetBrainsMono Nerd Font' \
  --speed 1.1 \
  --cols 100 \
  --rows 30
```

## Demo Flow Tips

- Press Enter twice after each Claude response for visual spacing
- Append `(brief)` to prompts for concise responses
- For JSON output, ask Claude to use `--format json`

## Key Capabilities

1. **Validation**: Ensures skills work across both Claude Code and Codex CLI
2. **Auto-Fix**: Automatically adds missing frontmatter for Codex compatibility
3. **Analysis**: Identifies token usage and optimization opportunities
4. **Bidirectional Sync**: Keeps configurations in sync between CLIs
5. **Dry-Run Support**: Preview changes before committing them

## Example Output Flow

```
You: Validate my skills for Codex

[Claude runs skrills validate --target codex]

Claude: I found 3 skills with issues:
- my-skill.md: Missing 'name' in frontmatter
- another-skill.md: Missing 'description' in frontmatter
- large-skill.md: Description exceeds 500 characters

Would you like me to auto-fix these issues?

You: Yes, fix them

[Claude runs skrills validate --target codex --autofix --backup]

Claude: Fixed all 3 issues. Backups created with .bak extension.
```
