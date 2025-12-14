# Playbooks and Demo Scripts

These scripted walkthroughs are for team training and validating new builds.

## Validation Workflow

Validate skills for cross-CLI compatibility:

```bash
# 1. Install skrills
./scripts/install.sh --client both

# 2. Validate all skills
skrills validate --target both

# 3. Fix Codex compatibility issues
skrills validate --target codex --autofix --backup

# 4. Verify fixes
skrills validate --target both --errors-only
```

## Sync Workflow

Sync configurations from Claude to Codex:

```bash
# 1. Preview changes
skrills sync-status --from claude

# 2. Sync everything
skrills sync-all --from claude --skip-existing-commands

# 3. Validate synced skills
skrills validate --target codex
```

## Analysis Workflow

Identify skills that need optimization:

```bash
# 1. Find large skills
skrills analyze --min-tokens 1000

# 2. Get optimization suggestions
skrills analyze --min-tokens 2000 --suggestions

# 3. Export for review
skrills analyze --format json > skills-analysis.json
```

## Claude Code Integration

```bash
# Install with Claude hook support
./scripts/install.sh --client claude

# Verify installation
skrills doctor

# Sync skills from Claude to Codex
skrills sync-all --from claude
```

## Codex Integration

```bash
# Install with Codex MCP support
./scripts/install.sh --client codex

# Verify MCP configuration
skrills doctor

# Validate skills for Codex
skrills validate --target codex
```

## TUI Workflow

Interactive sync and management:

```bash
# Launch TUI
skrills tui

# Navigate with arrow keys
# Select items with Space
# Confirm with Enter
```

## CI/CD Integration

Add to your CI pipeline:

```bash
# Fail on validation errors
skrills validate --target both --errors-only --format json

# Check for large skills
skrills analyze --min-tokens 3000 --format json
```
