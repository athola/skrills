# Brainstorm: GitHub Copilot CLI (~/.copilot) Integration

## Problem Statement

Skrills currently supports bidirectional sync between Claude Code (`~/.claude/`) and Codex CLI (`~/.codex/`). Users who also use GitHub Copilot CLI want the same validation, analysis, and multi-directional sync capabilities extended to `~/.copilot/`.

## Research Summary

### GitHub Copilot CLI Configuration Structure

Based on research from [GitHub Docs](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/use-copilot-cli), [GitHub Blog](https://github.blog/changelog/2026-01-14-github-copilot-cli-enhanced-agents-context-management-and-new-ways-to-install/), and [Custom Agents Configuration](https://docs.github.com/en/copilot/reference/custom-agents-configuration):

#### Directory Structure: `~/.copilot/`

| Path | Purpose | Format |
|------|---------|--------|
| `~/.copilot/config.json` | General settings (trusted_folders, allowed_urls, denied_urls, model) | JSON |
| `~/.copilot/mcp-config.json` | MCP server configurations | JSON |
| `~/.copilot/skills/<name>/SKILL.md` | **Skills (same as Codex!)** | Markdown with YAML frontmatter |
| `~/.copilot/agents/` | Custom agent definitions | Markdown with YAML frontmatter |
| `~/.copilot/copilot-instructions.md` | Global custom instructions | Markdown |

#### Hooks Configuration

> **Important Finding**: Unlike Claude Code, Copilot CLI hooks are **repository-level only**, not user-level. Hooks are stored in `.github/hooks/*.json` within each repository, not in `~/.copilot/hooks/`.

**Hook Location**: `.github/hooks/<hook-name>.json` (per-repository)

**Supported Hook Types**:

| Hook Type | Trigger |
|-----------|---------|
| `sessionStart` | When a Copilot session begins |
| `sessionEnd` | When a Copilot session ends |
| `userPromptSubmitted` | When user submits a prompt |
| `preToolUse` | Before a tool is invoked |
| `postToolUse` | After a tool completes |
| `errorOccurred` | When an error happens |

**Hook Schema** (`.github/hooks/<name>.json`):
```json
{
  "version": "1.0",
  "hooks": {
    "preToolUse": {
      "command": "bash -c 'echo Tool: $TOOL_NAME'",
      "tools": ["*"]
    },
    "postToolUse": {
      "command": "bash -c 'log-usage.sh'",
      "tools": ["write", "edit"]
    }
  }
}
```

**Implication for Skrills**: Hooks are NOT part of user-level config sync since they exist at repository level. This matches how Claude Code's project-level hooks (`.claude/hooks/`) work vs user-level (`~/.claude/hooks/`).

> **Key Discovery**: Copilot supports BOTH skills AND agents. Skills use the **exact same format as Codex** (`SKILL.md` in subdirectories with required `name` and `description` frontmatter). See [Agent Skills Documentation](https://docs.github.com/en/copilot/concepts/agents/about-agent-skills).

#### Environment Variables

| Variable | Purpose |
|----------|---------|
| `XDG_CONFIG_HOME` | Override config location (defaults to `~/.copilot`) |
| `COPILOT_GITHUB_TOKEN` | Authentication token |
| `COPILOT_MODEL` | Override default model |
| `GITHUB_TOKEN` | Fallback auth token |

#### config.json Schema (Inferred)

```json
{
  "trusted_folders": ["/path/to/trusted/dir"],
  "allowed_urls": ["https://example.com/*"],
  "denied_urls": ["https://blocked.com/*"],
  "model": "claude-sonnet-4"
}
```

#### mcp-config.json Schema

```json
{
  "mcpServers": {
    "server-name": {
      "type": "http",
      "url": "https://example.com/api/mcp",
      "headers": {},
      "tools": ["*"]
    }
  }
}
```

#### Custom Agent Schema (YAML Frontmatter)

```yaml
---
name: display-name
description: What this agent does (required)
target: vscode | github-copilot
tools: ["read", "edit", "search", "*"]
infer: true
mcp-servers:
  server-name:
    type: http
    url: https://...
metadata:
  key: value
---

# Agent Instructions

Markdown content (max 30,000 chars)
```

### Comparison Matrix

| Feature | Claude Code | Codex CLI | Copilot CLI |
|---------|-------------|-----------|-------------|
| **Config Root** | `~/.claude/` | `~/.codex/` | `~/.copilot/` |
| **Skills** | `skills/*.md` (permissive) | `skills/**/SKILL.md` (strict) | `skills/**/SKILL.md` (strict, same as Codex!) |
| **Agents** | N/A | N/A | `agents/*.md` (additional) |
| **Commands** | `commands/*.md` | `prompts/*.md` | N/A (uses agents) |
| **MCP Config** | `settings.json` | `config.json` | `mcp-config.json` (separate file) |
| **Settings File** | `settings.json` | `config.json` | `config.json` |
| **Custom Instructions** | `CLAUDE.md` (project) | N/A | `copilot-instructions.md` |
| **Skill Frontmatter** | Optional | Required (name, description) | Required (name, description) - same as Codex |
| **Feature Flags** | N/A | `config.toml [features]` | N/A |
| **Hooks (User)** | `~/.claude/hooks/*.md` | N/A | N/A (repo-level only) |
| **Hooks (Project)** | `.claude/hooks/*.md` | N/A | `.github/hooks/*.json` |

> **Similarity**: Copilot's skill format is **identical to Codex** - same `SKILL.md` naming, same required frontmatter fields (`name`, `description`), same directory structure. This dramatically simplifies the adapter implementation.

### Key Differences from Codex

1. **Separate MCP file**: Copilot uses `mcp-config.json` instead of embedding in `config.json`
2. **Additional agents concept**: Copilot has BOTH skills AND agents (agents are separate from skills)
3. **No commands directory**: Copilot uses agents for prompts, no separate prompts/commands
4. **URL filtering**: Copilot has explicit `allowed_urls`/`denied_urls` in config
5. **Trusted folders**: Copilot tracks trusted directories for security
6. **HTTP MCP servers**: Copilot supports HTTP-based MCP servers (not just stdio)
7. **No feature flags**: Copilot doesn't require `config.toml [features] skills = true`
8. **Hooks location**: Copilot hooks are repository-level only (`.github/hooks/`), not user-level

### Similarity to Codex (Major!)

1. **Skills format identical**: `~/.copilot/skills/<name>/SKILL.md` with same frontmatter
2. **Same frontmatter requirements**: `name` and `description` required
3. Both use JSON for main config
4. Both support MCP servers with similar schema
5. Both respect `XDG_CONFIG_HOME` for config location

---

## Approaches

### Approach 1: Full Parity Adapter

Create `CopilotAdapter` implementing `AgentAdapter` trait with full read/write support.

**Mapping Strategy:**
- `read_skills()` → reads from `~/.copilot/agents/*.md`
- `write_skills()` → writes to `~/.copilot/agents/<name>.agent.md`
- `read_mcp_servers()` → reads from `~/.copilot/mcp-config.json`
- `write_mcp_servers()` → writes to `~/.copilot/mcp-config.json`
- `read_preferences()` → reads model from `~/.copilot/config.json`
- `write_preferences()` → writes model to `~/.copilot/config.json`
- `read_commands()` → returns empty (Copilot doesn't have commands)
- `write_commands()` → no-op or convert to agents

**Pros:**
- Consistent with existing architecture
- Full bidirectional sync capability
- Reuses existing orchestrator

**Cons:**
- Commands concept doesn't map cleanly
- Agent schema differs significantly from skills
- May need schema transformation layer

### Approach 2: Agent-Centric Adapter

Treat Copilot as agent-only (no skills/commands separation). Add new `agents` field to `AgentAdapter` trait.

**Changes:**
```rust
pub trait AgentAdapter: Send + Sync {
    // Existing methods...

    fn read_agents(&self) -> Result<Vec<Agent>>;  // NEW
    fn write_agents(&self, agents: &[Agent]) -> Result<WriteReport>;  // NEW
}
```

**Pros:**
- Clean separation of agent concept
- More accurate model for Copilot
- Could benefit Claude/Codex if they add agents later

**Cons:**
- Breaks existing trait
- More invasive change
- Skills vs Agents confusion

### Approach 3: Transform Layer

Add transformation between Claude skills and Copilot agents during sync.

**Skill → Agent Transform:**
```rust
fn skill_to_agent(skill: &Command) -> CopilotAgent {
    let frontmatter = parse_frontmatter(&skill.content);
    CopilotAgent {
        name: frontmatter.name.unwrap_or(skill.name.clone()),
        description: frontmatter.description.unwrap_or_default(),
        tools: vec!["*".to_string()],  // Default to all tools
        content: strip_frontmatter(&skill.content),
    }
}
```

**Agent → Skill Transform:**
```rust
fn agent_to_skill(agent: &CopilotAgent) -> Command {
    let frontmatter = format!(
        "---\nname: {}\ndescription: {}\n---\n",
        agent.name, agent.description
    );
    Command {
        name: agent.name.clone(),
        content: format!("{}{}", frontmatter, agent.content).into_bytes(),
        // ...
    }
}
```

**Pros:**
- Minimal trait changes
- Explicit transformation logic
- Easy to customize per-direction

**Cons:**
- Some information loss possible
- Complex bidirectional transforms
- Tool configuration doesn't map to skills

### Approach 4: Near-Clone of CodexAdapter (Recommended)

Since Copilot uses the **exact same skill format** as Codex, we can implement `CopilotAdapter` as a near-clone of `CodexAdapter` with minor modifications.

**Implementation:**
1. `read_skills()` / `write_skills()` → Same logic as Codex (`~/.copilot/skills/**/SKILL.md`)
2. `read_mcp_servers()` → Read from `~/.copilot/mcp-config.json` (not config.json)
3. `write_mcp_servers()` → Write to `~/.copilot/mcp-config.json`
4. `read_preferences()` → Read model from `~/.copilot/config.json`
5. `write_preferences()` → Write model to `~/.copilot/config.json`
6. `read_commands()` → Return empty (Copilot doesn't have commands)
7. `write_commands()` → No-op
8. **No feature flag needed** (unlike Codex's `config.toml [features] skills = true`)

**File Mapping:**

| Skrills Concept | Copilot Path | Format |
|-----------------|--------------|--------|
| Skills | `~/.copilot/skills/<name>/SKILL.md` | MD + YAML (same as Codex!) |
| MCP Servers | `~/.copilot/mcp-config.json` | JSON (separate file) |
| Preferences | `~/.copilot/config.json` | JSON |
| Commands | N/A (not supported) | - |

**Why This Works:**
- Skills are 100% compatible with Codex format
- Same validation rules apply (required name/description)
- Same discovery logic works
- Only MCP config location differs

---

## Constraints

### Technical Constraints

1. **Rust trait compatibility**: Must implement `AgentAdapter` without breaking changes
2. **Byte-safe content**: Maintain `Vec<u8>` storage for non-UTF-8 content
3. **Hash-based change detection**: Use SHA256 for skip-unchanged logic
4. **XDG compliance**: Honor `XDG_CONFIG_HOME` for config location

### Schema Constraints

1. **Required description**: Copilot agents MUST have description field
2. **Max content length**: 30,000 chars for agent markdown content
3. **Tool access patterns**: Copilot uses different tool naming (read, edit, search vs MCP tools)
4. **No SKILL.md convention**: Copilot uses `*.md` or `*.agent.md` in flat directory

### Behavioral Constraints

1. **No commands**: Copilot doesn't have slash commands concept
2. **HTTP MCP servers**: Copilot supports HTTP servers, not just stdio
3. **URL filtering**: Must preserve `allowed_urls`/`denied_urls` on write
4. **Trusted folders**: Must preserve `trusted_folders` on write

---

## Goals

### Primary Goals

1. **Validate Copilot agents**: Check agent files for schema compliance
2. **Analyze agent content**: Token counting, dependency detection
3. **Sync to/from Copilot**: Bidirectional sync with Claude and Codex
4. **Unified MCP management**: Sync MCP servers across all three CLIs

### Secondary Goals

1. **Agent discovery**: Include `~/.copilot/agents/` in discovery sources
2. **Model transformation**: Map Claude/GPT models appropriately
3. **Preference sync**: Sync common settings (model, etc.)

### Non-Goals (Out of Scope)

1. Syncing `trusted_folders` (security-sensitive, user-specific)
2. Syncing URL filters (security-sensitive, user-specific)
3. Creating new agent types (beyond skill conversion)
4. IDE-specific configurations

---

## Trade-offs

### Skills vs Agents Mapping

| Aspect | Skill-as-Agent | Separate Concepts |
|--------|----------------|-------------------|
| Simplicity | Higher | Lower |
| Accuracy | Lower (some loss) | Higher |
| Maintenance | Lower | Higher |
| User mental model | May confuse | Clearer |

**Decision**: Use skill-as-agent mapping for simplicity. Document differences.

### MCP Server Schema Differences

| Aspect | Unified Schema | Per-CLI Schema |
|--------|---------------|----------------|
| Code complexity | Lower | Higher |
| Feature support | Common subset | Full features |
| Sync accuracy | Lossy | Lossless |

**Decision**: Use unified schema with optional per-CLI extensions.

### Commands Handling

| Approach | Pros | Cons |
|----------|------|------|
| Ignore entirely | Simple | Incomplete sync |
| Convert to agents | Feature parity | May create spam |
| Separate sync flag | User control | Complexity |

**Decision**: Ignore commands for Copilot. Return empty, no-op write.

---

## Model Transformation

### Copilot ↔ Claude Model Mapping

```rust
// Copilot → Claude
"claude-sonnet-4" → "sonnet"
"gpt-5" → "opus"  // best available
"gpt-5-mini" → "sonnet"
"gpt-4.1" → "sonnet"
"gpt-4o" → "opus"
"gpt-4o-mini" → "sonnet"

// Claude → Copilot
"opus" → "claude-sonnet-4"  // Copilot default
"sonnet" → "claude-sonnet-4"
"haiku" → "gpt-5-mini"  // lightweight option
```

### Copilot ↔ Codex Model Mapping

```rust
// Same models available, direct mapping
"gpt-5" ↔ "gpt-4o" (upgrade path)
"gpt-5-mini" ↔ "gpt-4o-mini"
```

---

## Implementation Plan

### Phase 1: CopilotAdapter (Core)

1. Create `crates/sync/src/adapters/copilot.rs`
2. Implement `AgentAdapter` trait
3. Handle `~/.copilot/agents/` for skills
4. Handle `~/.copilot/mcp-config.json` for MCP servers
5. Handle `~/.copilot/config.json` for preferences
6. Add model transformation logic

### Phase 2: Validation Rules

1. Add Copilot-specific validation target in `validate` crate
2. Validate agent frontmatter (required description)
3. Validate content length (30,000 char limit)
4. Check tool access patterns

### Phase 3: Discovery Integration

1. Add `SkillSource::Copilot` variant
2. Update discovery to scan `~/.copilot/agents/`
3. Add priority configuration for Copilot source

### Phase 4: CLI & MCP Tools

1. Add `--from copilot` and `--to copilot` flags
2. Expose `sync-from-copilot`, `sync-to-copilot` MCP tools
3. Update `sync-all` to include Copilot
4. Add `validate-copilot-agents` tool

### Phase 5: Documentation & Testing

1. Add comprehensive tests for CopilotAdapter
2. Update README with Copilot support
3. Add migration guide for Copilot users
4. Document schema differences

---

## Questions for User

1. **Discovery Priority**: Where should `~/.copilot/skills/` rank in skill discovery?
   - Before Codex? After Codex? Same level?
   - Suggested: Same priority as Codex (both are "strict" format)

2. **Commands Handling**: When syncing FROM Claude to Copilot:
   - Skip commands entirely (Copilot doesn't support them)?
   - Convert to Copilot agents (different concept)?
   - Suggested: Skip commands (clean separation)

3. **HTTP MCP Servers**: Copilot supports HTTP-based MCP servers. Claude/Codex expect stdio.
   - Filter out HTTP servers when syncing to Claude/Codex?
   - Attempt conversion (may not work)?
   - Suggested: Preserve in Copilot, filter when syncing out

4. **Agents Support**: Should Skrills also sync Copilot's custom agents (`~/.copilot/agents/`)?
   - These are separate from skills with different schema
   - Could add `read_agents()` / `write_agents()` to trait
   - Suggested: Phase 2 - focus on skills first

---

## Next Steps

1. [ ] Get user feedback on approach selection
2. [ ] Create `CopilotAdapter` implementing `AgentAdapter`
3. [ ] Add validation rules for Copilot agent schema
4. [ ] Integrate into discovery system
5. [ ] Update CLI and MCP server tools
6. [ ] Write comprehensive tests
7. [ ] Update documentation

---

## Sources

- [GitHub Copilot CLI Documentation](https://docs.github.com/en/copilot/how-tos/use-copilot-agents/use-copilot-cli)
- [About GitHub Copilot CLI](https://docs.github.com/en/copilot/concepts/agents/about-copilot-cli)
- [**About Agent Skills**](https://docs.github.com/en/copilot/concepts/agents/about-agent-skills) - Key documentation showing `~/.copilot/skills/` support
- [Custom Agents Configuration](https://docs.github.com/en/copilot/reference/custom-agents-configuration)
- [GitHub Copilot CLI Changelog](https://github.blog/changelog/2026-01-14-github-copilot-cli-enhanced-agents-context-management-and-new-ways-to-install/)
- [Agentic Memory for GitHub Copilot](https://github.blog/changelog/2026-01-15-agentic-memory-for-github-copilot-is-in-public-preview/)
- [**About Hooks**](https://docs.github.com/en/copilot/concepts/agents/coding-agent/about-hooks) - Repository-level hooks documentation (`.github/hooks/`)
