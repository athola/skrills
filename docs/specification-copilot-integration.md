# GitHub Copilot CLI Integration - Specification v0.1.0

**Author**: Claude (via Skrills brainstorm)
**Date**: 2026-01-17
**Status**: Draft

## Change History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1.0 | 2026-01-17 | Claude | Initial draft from brainstorm |

---

## Overview

**Purpose**: Extend Skrills to support GitHub Copilot CLI (`~/.copilot/`) with bidirectional sync, validation, and analysis capabilities, matching existing support for Claude Code (`~/.claude/`) and Codex CLI (`~/.codex/`).

**Scope**:
- **IN**: CopilotAdapter implementation, skill sync, MCP server sync, preference sync, skill discovery, validation rules
- **OUT**: Repository-level hooks, trusted_folders sync, URL filter sync, IDE-specific configs, custom agents (Phase 2)

**Stakeholders**:
- **Multi-CLI Users**: Developers using multiple AI coding assistants who want unified configuration management
- **Skrills Maintainers**: Need consistent adapter pattern and comprehensive test coverage
- **Copilot CLI Users**: Want validation and analysis tools for their configurations

---

## Functional Requirements

### FR-001: CopilotAdapter Implementation

**Description**: Create a new `CopilotAdapter` struct implementing the `AgentAdapter` trait to enable read/write operations on `~/.copilot/` configuration.

**Acceptance Criteria**:
- [ ] Given `~/.copilot/` directory exists, when `CopilotAdapter::new()` is called, then adapter initializes with correct config root
- [ ] Given `XDG_CONFIG_HOME` is set, when `CopilotAdapter::config_root()` is called, then returns `$XDG_CONFIG_HOME/copilot` instead of `~/.copilot`
- [ ] Given adapter exists, when `name()` is called, then returns `"copilot"`
- [ ] Given adapter exists, when `supported_fields()` is called, then returns `FieldSupport { commands: false, mcp_servers: true, preferences: true, skills: true }`

**Priority**: High
**Dependencies**: None
**Estimated Effort**: M

---

### FR-002: Read Skills from Copilot

**Description**: Read skills from `~/.copilot/skills/<name>/SKILL.md` using the same format as Codex (markdown with required YAML frontmatter containing `name` and `description`).

**Acceptance Criteria**:
- [ ] Given `~/.copilot/skills/my-skill/SKILL.md` exists with valid frontmatter, when `read_skills()` is called, then skill is returned with correct name, description, and content
- [ ] Given skill file has missing `name` frontmatter, when `read_skills()` is called, then skill is excluded with warning logged
- [ ] Given skill file has missing `description` frontmatter, when `read_skills()` is called, then skill is excluded with warning logged
- [ ] Given `~/.copilot/skills/` directory does not exist, when `read_skills()` is called, then returns empty Vec without error
- [ ] Given nested skill structure `~/.copilot/skills/category/my-skill/SKILL.md`, when `read_skills()` is called, then skill is discovered and returned

**Priority**: High
**Dependencies**: FR-001
**Estimated Effort**: M

---

### FR-003: Write Skills to Copilot

**Description**: Write skills to `~/.copilot/skills/<name>/SKILL.md` format, creating directories as needed.

**Acceptance Criteria**:
- [ ] Given valid skill with name and description, when `write_skills()` is called, then file is created at `~/.copilot/skills/<name>/SKILL.md`
- [ ] Given skill content has YAML frontmatter, when writing, then frontmatter is preserved exactly
- [ ] Given skill with same content hash already exists, when `write_skills()` is called, then file is skipped (skip-unchanged)
- [ ] Given `~/.copilot/skills/` directory does not exist, when `write_skills()` is called, then directory is created
- [ ] Given write operation completes, when `WriteReport` is returned, then report contains accurate created/updated/skipped counts

**Priority**: High
**Dependencies**: FR-001, FR-002
**Estimated Effort**: M

---

### FR-004: Read MCP Servers from Copilot

**Description**: Read MCP server configurations from `~/.copilot/mcp-config.json` (separate file from main config).

**Acceptance Criteria**:
- [ ] Given `~/.copilot/mcp-config.json` exists with valid JSON, when `read_mcp_servers()` is called, then servers are returned as HashMap
- [ ] Given JSON has `mcpServers` key with server entries, when parsing, then each server is correctly mapped to `McpServer` struct
- [ ] Given server has `type: "http"`, when parsing, then `McpServer.transport` is set to HTTP variant
- [ ] Given `~/.copilot/mcp-config.json` does not exist, when `read_mcp_servers()` is called, then returns empty HashMap without error
- [ ] Given malformed JSON, when `read_mcp_servers()` is called, then returns appropriate error

**Priority**: High
**Dependencies**: FR-001
**Estimated Effort**: S

---

### FR-005: Write MCP Servers to Copilot

**Description**: Write MCP server configurations to `~/.copilot/mcp-config.json`.

**Acceptance Criteria**:
- [ ] Given valid MCP servers HashMap, when `write_mcp_servers()` is called, then file is created/updated at `~/.copilot/mcp-config.json`
- [ ] Given servers map, when writing, then JSON has `mcpServers` top-level key
- [ ] Given HTTP transport server, when writing, then `type: "http"` is included in output
- [ ] Given stdio transport server, when writing, then appropriate stdio fields are written
- [ ] Given same servers already exist with same hash, when writing, then file is skipped

**Priority**: High
**Dependencies**: FR-001, FR-004
**Estimated Effort**: S

---

### FR-006: Read Preferences from Copilot

**Description**: Read user preferences (model selection) from `~/.copilot/config.json`.

**Acceptance Criteria**:
- [ ] Given `~/.copilot/config.json` exists with `model` field, when `read_preferences()` is called, then `Preferences.model` contains the value
- [ ] Given `~/.copilot/config.json` does not exist, when `read_preferences()` is called, then returns default Preferences
- [ ] Given config has other fields (`trusted_folders`, `allowed_urls`), when parsing, then only `model` is extracted (other fields preserved for writes)
- [ ] Given malformed JSON, when `read_preferences()` is called, then returns appropriate error

**Priority**: Medium
**Dependencies**: FR-001
**Estimated Effort**: S

---

### FR-007: Write Preferences to Copilot

**Description**: Write user preferences to `~/.copilot/config.json`, preserving existing fields.

**Acceptance Criteria**:
- [ ] Given Preferences with model value, when `write_preferences()` is called, then `model` field is updated in config.json
- [ ] Given existing config.json with `trusted_folders`, when writing, then `trusted_folders` is preserved unchanged
- [ ] Given existing config.json with `allowed_urls`/`denied_urls`, when writing, then URL fields are preserved unchanged
- [ ] Given `~/.copilot/config.json` does not exist, when `write_preferences()` is called, then file is created with model field only

**Priority**: Medium
**Dependencies**: FR-001, FR-006
**Estimated Effort**: S

---

### FR-008: Commands No-Op Implementation

**Description**: Implement `read_commands()` and `write_commands()` as no-ops since Copilot does not support slash commands.

**Acceptance Criteria**:
- [ ] Given adapter, when `read_commands(include_marketplace)` is called, then returns empty Vec
- [ ] Given commands list, when `write_commands()` is called, then returns `WriteReport` with zero operations and no files modified
- [ ] Given `supported_fields()`, when checking commands, then returns `false`

**Priority**: Low
**Dependencies**: FR-001
**Estimated Effort**: XS

---

### FR-009: Skill Discovery Integration

**Description**: Add Copilot as a skill discovery source in the discovery system.

**Acceptance Criteria**:
- [ ] Given `SkillSource` enum, when adding Copilot variant, then `SkillSource::Copilot` is available
- [ ] Given discovery runs, when `~/.copilot/skills/` contains skills, then skills are discovered with source attribution
- [ ] Given Copilot and Codex have same skill name, when resolving priority, then priority configuration determines winner
- [ ] Given Copilot skill, when displaying source, then shows "copilot" identifier

**Priority**: Medium
**Dependencies**: FR-002
**Estimated Effort**: M

---

### FR-010: Copilot-Specific Validation

**Description**: Add validation rules specific to Copilot skill and agent schemas.

**Acceptance Criteria**:
- [ ] Given skill without `name` frontmatter, when validating for Copilot, then validation fails with clear error message
- [ ] Given skill without `description` frontmatter, when validating for Copilot, then validation fails with clear error message
- [ ] Given skill content exceeding 30,000 characters, when validating for Copilot, then validation warns about limit
- [ ] Given valid skill with required fields, when validating for Copilot, then validation passes

**Priority**: Medium
**Dependencies**: FR-002
**Estimated Effort**: S

---

### FR-011: CLI Integration

**Description**: Add `--from copilot` and `--to copilot` flags to sync commands.

**Acceptance Criteria**:
- [ ] Given `skrills sync --from copilot --to claude`, when executed, then skills sync from Copilot to Claude
- [ ] Given `skrills sync --from claude --to copilot`, when executed, then skills sync from Claude to Copilot
- [ ] Given `skrills sync-all`, when executed with Copilot adapter registered, then Copilot is included in multi-way sync
- [ ] Given `skrills validate --target copilot`, when executed, then Copilot-specific validation runs

**Priority**: High
**Dependencies**: FR-001 through FR-010
**Estimated Effort**: M

---

### FR-012: MCP Server Tool Exposure

**Description**: Expose `sync-from-copilot` and `sync-to-copilot` as MCP tools.

**Acceptance Criteria**:
- [ ] Given MCP server running, when listing tools, then `sync-from-copilot` and `sync-to-copilot` are available
- [ ] Given MCP tool invoked with valid params, when executing, then sync operation completes and returns result
- [ ] Given sync tool, when errors occur, then error is returned in MCP response format

**Priority**: Medium
**Dependencies**: FR-011
**Estimated Effort**: S

---

## Non-Functional Requirements

### NFR-001: Performance - Skill Discovery

**Requirement**: Skill discovery from `~/.copilot/skills/` completes within acceptable time bounds.

**Measurement**:
- Metric: Discovery time for 100 skills
- Target: < 500ms on standard hardware
- Tool: Integration test with timing

**Priority**: Medium

---

### NFR-002: Performance - Sync Operations

**Requirement**: Sync operations between adapters complete efficiently using hash-based skip-unchanged logic.

**Measurement**:
- Metric: Sync time when no changes exist (100 skills)
- Target: < 100ms (skip-unchanged optimization)
- Tool: Integration test with timing

**Priority**: Medium

---

### NFR-003: Reliability - Error Handling

**Requirement**: Adapter gracefully handles missing directories, malformed files, and permission errors.

**Measurement**:
- Metric: Error scenarios handled without panic
- Target: 100% of error paths return Result::Err with descriptive message
- Tool: Unit tests covering error scenarios

**Priority**: High

---

### NFR-004: Maintainability - Code Reuse

**Requirement**: CopilotAdapter reuses common code from CodexAdapter where possible to minimize duplication.

**Measurement**:
- Metric: Shared utility functions for skill parsing/writing
- Target: < 30% code duplication between CodexAdapter and CopilotAdapter
- Tool: Code review, duplication analysis

**Priority**: Medium

---

### NFR-005: Compatibility - XDG Compliance

**Requirement**: Adapter respects XDG Base Directory specification for config location.

**Measurement**:
- Metric: Config root derived from `XDG_CONFIG_HOME` when set
- Target: 100% compliance with XDG spec
- Tool: Unit tests with environment variable overrides

**Priority**: High

---

### NFR-006: Security - Field Preservation

**Requirement**: Security-sensitive fields in Copilot config are preserved but not synced.

**Measurement**:
- Metric: `trusted_folders`, `allowed_urls`, `denied_urls` preserved on write
- Target: Never overwrite or sync security fields
- Tool: Integration tests verifying field preservation

**Priority**: Critical

---

## Technical Constraints

### Technology Stack

- **Language**: Rust (match existing codebase)
- **Trait Implementation**: Must implement `AgentAdapter` without breaking changes
- **Async**: All I/O operations use async/await pattern
- **Error Handling**: Use `anyhow::Result` consistent with codebase
- **Testing**: Unit tests with mockall, integration tests with tempdir

### Integration Points

| Integration | Protocol | Notes |
|-------------|----------|-------|
| `AgentAdapter` trait | Rust trait | Implement all required methods |
| `WriteReport` | Struct | Use existing report format |
| `Command` | Struct | Use for skills (same as Codex) |
| `McpServer` | Struct | Use existing MCP server types |
| Discovery system | `SkillSource` enum | Add new variant |
| CLI | clap args | Add new flags |
| MCP Server | Tool definitions | Add new tools |

### Data Requirements

| Data | Schema | Storage |
|------|--------|---------|
| Skills | YAML frontmatter + MD | `~/.copilot/skills/<name>/SKILL.md` |
| MCP Servers | JSON | `~/.copilot/mcp-config.json` |
| Preferences | JSON | `~/.copilot/config.json` |
| Hash Cache | SHA256 | In-memory per operation |

### Deployment

- No deployment changes (library code)
- Tests must pass in CI
- Documentation updates required

---

## Out of Scope (v1.0)

| Feature | Rationale |
|---------|-----------|
| Repository-level hooks sync | Hooks in `.github/hooks/` are per-repo, not user-level config |
| `trusted_folders` sync | Security-sensitive, user-specific |
| URL filter (`allowed_urls`/`denied_urls`) sync | Security-sensitive, user-specific |
| Custom agents (`~/.copilot/agents/`) sync | Different schema from skills, defer to Phase 2 |
| IDE-specific configurations | Out of Skrills scope |
| `copilot-instructions.md` sync | No equivalent in Claude/Codex |
| HTTP ↔ stdio MCP conversion | Transport types not compatible |

---

## Dependencies

| Dependency | Type | Status |
|------------|------|--------|
| `AgentAdapter` trait | Internal | Exists, no changes needed |
| `CodexAdapter` | Internal | Reference implementation |
| `serde_json` | External | Already in deps |
| `serde_yaml` | External | Already in deps |
| `sha2` | External | Already in deps |
| `walkdir` | External | Already in deps |

---

## Acceptance Testing Strategy

### Unit Tests

- Test each `AgentAdapter` method in isolation
- Mock filesystem with tempdir
- Test edge cases: empty dirs, missing files, malformed content
- Test XDG_CONFIG_HOME override

### Integration Tests

- End-to-end sync: Copilot → Claude
- End-to-end sync: Claude → Copilot
- Sync with Codex (three-way)
- Validation with Copilot target
- Discovery including Copilot source

### Validation Tests

- Missing frontmatter fields
- Content length limits
- Required field presence

---

## Success Criteria

- [ ] All High priority FRs implemented and tested
- [ ] `CopilotAdapter` passes same test patterns as `CodexAdapter`
- [ ] Sync between Copilot, Claude, and Codex works bidirectionally
- [ ] No regressions in existing Claude/Codex functionality
- [ ] Documentation updated with Copilot support
- [ ] CI passes with new tests

---

## Glossary

| Term | Definition |
|------|------------|
| **Adapter** | Implementation of `AgentAdapter` trait for a specific CLI |
| **CLI** | Command-line interface (Claude Code, Codex, Copilot) |
| **Frontmatter** | YAML metadata at top of markdown file between `---` delimiters |
| **MCP** | Model Context Protocol - standardized server interface |
| **Skill** | Reusable capability defined in markdown with frontmatter |
| **Skip-unchanged** | Optimization that skips writing files with identical content hash |
| **XDG** | XDG Base Directory Specification for config file locations |

---

## References

- [Brainstorm Document](./brainstorm-copilot-integration.md)
- [AgentAdapter Trait](../crates/sync/src/adapters/traits.rs)
- [CodexAdapter Implementation](../crates/sync/src/adapters/codex.rs)
- [ClaudeAdapter Implementation](../crates/sync/src/adapters/claude.rs)
- [GitHub Copilot CLI Documentation](https://docs.github.com/en/copilot/concepts/agents/about-copilot-cli)
- [About Agent Skills](https://docs.github.com/en/copilot/concepts/agents/about-agent-skills)
- [About Hooks](https://docs.github.com/en/copilot/concepts/agents/coding-agent/about-hooks)
