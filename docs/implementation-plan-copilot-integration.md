# GitHub Copilot CLI Integration - Implementation Plan v0.1.0

**Author**: Claude (via Skrills planning)
**Date**: 2026-01-17
**Sprint Length**: 1 week
**Team Size**: 1 (solo developer)
**Target Completion**: ~2 sprints

---

## Architecture

### System Overview

The Copilot integration follows the existing adapter pattern used for Claude and Codex. A new `CopilotAdapter` implements the `AgentAdapter` trait, enabling bidirectional sync of skills, MCP servers, and preferences between all three CLI tools.

```
┌─────────────────────────────────────────────────────────────────┐
│                         Skrills Core                            │
├─────────────────────────────────────────────────────────────────┤
│  Orchestrator                                                   │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐          │
│  │ClaudeAdapter │  │ CodexAdapter │  │CopilotAdapter│ ← NEW    │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘          │
│         │                 │                 │                   │
│         ▼                 ▼                 ▼                   │
│  ~/.claude/          ~/.codex/         ~/.copilot/              │
│  ├─ skills/*.md      ├─ skills/**/     ├─ skills/**/            │
│  │                   │  SKILL.md       │  SKILL.md              │
│  ├─ settings.json    ├─ config.json    ├─ config.json           │
│  │  (mcp, prefs)     │  (mcp, prefs)   │  (prefs only)          │
│  └─ commands/*.md    └─ prompts/*.md   └─ mcp-config.json ← KEY │
│                                           (mcp servers)         │
└─────────────────────────────────────────────────────────────────┘
```

### Key Architectural Decision

**Copilot uses separate file for MCP servers**: Unlike Claude (`settings.json`) and Codex (`config.json`) which embed MCP servers in the main config, Copilot uses a dedicated `mcp-config.json`. This requires different file paths in `read_mcp_servers()` and `write_mcp_servers()`.

### Component Diagram

```
┌──────────────────────────────────────────────────────────────────┐
│ crates/sync/src/adapters/                                        │
│ ┌────────────────┐                                               │
│ │   traits.rs    │ ← AgentAdapter trait (unchanged)              │
│ └───────┬────────┘                                               │
│         │ implements                                             │
│         ▼                                                        │
│ ┌────────────────┐  ┌────────────────┐  ┌────────────────┐      │
│ │  claude.rs     │  │   codex.rs     │  │  copilot.rs    │ NEW  │
│ │ ClaudeAdapter  │  │  CodexAdapter  │  │ CopilotAdapter │      │
│ └────────────────┘  └────────────────┘  └────────────────┘      │
│                                               │                  │
│                                               ▼                  │
│                                    ┌────────────────────┐       │
│                                    │ Copilot-specific:  │       │
│                                    │ • mcp-config.json  │       │
│                                    │ • No config.toml   │       │
│                                    │ • No commands      │       │
│                                    └────────────────────┘       │
└──────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────┐
│ crates/validate/src/                                             │
│ ┌────────────────┐                                               │
│ │  targets.rs    │ ← Add ValidationTarget::Copilot               │
│ └────────────────┘                                               │
│ ┌────────────────┐                                               │
│ │  rules.rs      │ ← Add Copilot-specific validation             │
│ └────────────────┘                                               │
└──────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────┐
│ crates/discovery/src/                                            │
│ ┌────────────────┐                                               │
│ │  sources.rs    │ ← Add SkillSource::Copilot variant            │
│ └────────────────┘                                               │
└──────────────────────────────────────────────────────────────────┘
```

### Components

#### Component: CopilotAdapter

**Responsibility**: Read/write Copilot CLI configuration from `~/.copilot/`

**Technology**: Rust, serde_json, walkdir

**Interfaces**:
- `AgentAdapter` trait: All standard read/write methods
- `with_root(PathBuf)`: Constructor for testing

**Dependencies**:
- `traits.rs`: AgentAdapter trait
- `common.rs`: Command, McpServer, Preferences types
- `report.rs`: WriteReport, SkipReason types

**Data**:
- Skills: `~/.copilot/skills/<name>/SKILL.md`
- MCP Servers: `~/.copilot/mcp-config.json`
- Preferences: `~/.copilot/config.json`

**Key Differences from CodexAdapter**:
1. MCP servers read/write from `mcp-config.json` (not `config.json`)
2. No `config.toml` feature flag management
3. `read_commands()` returns empty Vec
4. `write_commands()` is no-op
5. Preserve security fields on preference writes

---

## Task Breakdown

### Phase 1: Core Adapter (Sprint 1) - TASK-001 through TASK-007

#### TASK-001: Create CopilotAdapter Scaffold

**Description**: Create `crates/sync/src/adapters/copilot.rs` with basic struct and trait implementation returning defaults/empty.

**Type**: Implementation
**Priority**: P0 (Critical)
**Estimate**: 2 points
**Dependencies**: None
**Sprint**: Sprint 1

**Linked Requirements**: FR-001

**Acceptance Criteria**:
- [ ] File `copilot.rs` exists in adapters directory
- [ ] `CopilotAdapter` struct with `root: PathBuf` field
- [ ] `new()` constructor using `dirs::home_dir()` + `.copilot`
- [ ] `with_root(PathBuf)` constructor for testing
- [ ] `name()` returns `"copilot"`
- [ ] `config_root()` returns `self.root.clone()`
- [ ] `supported_fields()` returns correct flags
- [ ] Module exported from `adapters/mod.rs`
- [ ] Tests for constructors

**Technical Notes**:
- Copy structure from `codex.rs` as starting point
- Remove all Codex-specific logic initially
- Stub all trait methods with empty/default returns

---

#### TASK-002: Implement read_skills()

**Description**: Implement skill reading from `~/.copilot/skills/<name>/SKILL.md` (identical to Codex).

**Type**: Implementation
**Priority**: P0 (Critical)
**Estimate**: 3 points
**Dependencies**: TASK-001
**Sprint**: Sprint 1

**Linked Requirements**: FR-002

**Acceptance Criteria**:
- [ ] Reads from `~/.copilot/skills/` directory
- [ ] Discovers `**/SKILL.md` pattern
- [ ] Extracts skill name from parent directory path
- [ ] Skips hidden directories/files
- [ ] Returns empty Vec if directory doesn't exist
- [ ] Computes SHA256 hash of content
- [ ] Sets source_path correctly
- [ ] Unit tests covering all criteria

**Technical Notes**:
- Reuse logic from `CodexAdapter::read_skills()`
- Use `walkdir` for recursive discovery
- Use `is_hidden_path()` helper pattern

---

#### TASK-003: Implement write_skills()

**Description**: Implement skill writing to `~/.copilot/skills/<name>/SKILL.md` with skip-unchanged optimization.

**Type**: Implementation
**Priority**: P0 (Critical)
**Estimate**: 3 points
**Dependencies**: TASK-001, TASK-002
**Sprint**: Sprint 1

**Linked Requirements**: FR-003

**Acceptance Criteria**:
- [ ] Creates `~/.copilot/skills/` directory if missing
- [ ] Writes skills to `skills/<name>/SKILL.md`
- [ ] Uses `sanitize_name()` for path safety
- [ ] Skips files with matching hash (skip-unchanged)
- [ ] Returns accurate WriteReport counts
- [ ] NO config.toml manipulation (unlike Codex)
- [ ] Unit tests for create, update, skip scenarios

**Technical Notes**:
- Reuse logic from `CodexAdapter::write_skills()`
- Remove `ensure_skills_feature_flag_enabled()` call
- Keep `sanitize_name()` for security

---

#### TASK-004: Implement read_mcp_servers()

**Description**: Read MCP server configurations from `~/.copilot/mcp-config.json`.

**Type**: Implementation
**Priority**: P0 (Critical)
**Estimate**: 2 points
**Dependencies**: TASK-001
**Sprint**: Sprint 1

**Linked Requirements**: FR-004

**Acceptance Criteria**:
- [ ] Reads from `~/.copilot/mcp-config.json` (NOT config.json)
- [ ] Parses `mcpServers` object from JSON
- [ ] Maps each server to `McpServer` struct
- [ ] Returns empty HashMap if file doesn't exist
- [ ] Returns error on malformed JSON
- [ ] Handles HTTP-type servers (stores as-is for now)
- [ ] Unit tests for existing, missing, malformed file

**Technical Notes**:
- Different file path from Codex
- Same JSON structure inside (mcpServers key)
- HTTP server support is read-only for now

---

#### TASK-005: Implement write_mcp_servers()

**Description**: Write MCP server configurations to `~/.copilot/mcp-config.json`.

**Type**: Implementation
**Priority**: P0 (Critical)
**Estimate**: 2 points
**Dependencies**: TASK-001, TASK-004
**Sprint**: Sprint 1

**Linked Requirements**: FR-005

**Acceptance Criteria**:
- [ ] Writes to `~/.copilot/mcp-config.json` (NOT config.json)
- [ ] Creates file with `mcpServers` wrapper object
- [ ] Preserves existing MCP config structure if file exists
- [ ] Uses pretty-printed JSON output
- [ ] Returns accurate WriteReport
- [ ] Unit tests for create, update scenarios

**Technical Notes**:
- Separate file from preferences (config.json)
- Match Codex JSON structure for compatibility

---

#### TASK-006: Implement read_preferences() and write_preferences()

**Description**: Read/write model preference from `~/.copilot/config.json`, preserving security fields.

**Type**: Implementation
**Priority**: P1 (High)
**Estimate**: 3 points
**Dependencies**: TASK-001
**Sprint**: Sprint 1

**Linked Requirements**: FR-006, FR-007

**Acceptance Criteria**:
- [ ] Reads `model` field from `~/.copilot/config.json`
- [ ] Returns default Preferences if file missing
- [ ] Returns error on malformed JSON
- [ ] Writes only `model` field
- [ ] Preserves `trusted_folders` on write
- [ ] Preserves `allowed_urls` on write
- [ ] Preserves `denied_urls` on write
- [ ] Creates file if missing with model only
- [ ] Unit tests for preservation of security fields

**Technical Notes**:
- Critical: Never overwrite security fields
- Read existing JSON, modify only model, write back

---

#### TASK-007: Implement no-op commands

**Description**: Implement `read_commands()` and `write_commands()` as no-ops.

**Type**: Implementation
**Priority**: P3 (Low)
**Estimate**: 1 point
**Dependencies**: TASK-001
**Sprint**: Sprint 1

**Linked Requirements**: FR-008

**Acceptance Criteria**:
- [ ] `read_commands()` returns empty Vec
- [ ] `write_commands()` returns WriteReport with zeros
- [ ] `supported_fields().commands` is `false`
- [ ] Unit tests verify behavior

---

### Phase 2: Integration (Sprint 2) - TASK-008 through TASK-014

#### TASK-008: Add SkillSource::Copilot variant

**Description**: Add Copilot to skill discovery system.

**Type**: Implementation
**Priority**: P1 (High)
**Estimate**: 2 points
**Dependencies**: TASK-002
**Sprint**: Sprint 2

**Linked Requirements**: FR-009

**Acceptance Criteria**:
- [ ] `SkillSource::Copilot` variant exists
- [ ] Discovery scans `~/.copilot/skills/`
- [ ] Skills have correct source attribution
- [ ] Integration tests verify discovery

---

#### TASK-009: Add Copilot validation target

**Description**: Add `ValidationTarget::Copilot` with Copilot-specific rules.

**Type**: Implementation
**Priority**: P1 (High)
**Estimate**: 3 points
**Dependencies**: TASK-002
**Sprint**: Sprint 2

**Linked Requirements**: FR-010

**Acceptance Criteria**:
- [ ] `ValidationTarget::Copilot` variant exists
- [ ] Validates required `name` frontmatter
- [ ] Validates required `description` frontmatter
- [ ] Warns on content > 30,000 chars
- [ ] Unit tests for validation rules

---

#### TASK-010: Add CLI flags for Copilot sync

**Description**: Add `--from copilot` and `--to copilot` flags to sync commands.

**Type**: Implementation
**Priority**: P0 (Critical)
**Estimate**: 2 points
**Dependencies**: TASK-001 through TASK-007
**Sprint**: Sprint 2

**Linked Requirements**: FR-011

**Acceptance Criteria**:
- [ ] `skrills sync --from copilot --to claude` works
- [ ] `skrills sync --from claude --to copilot` works
- [ ] `skrills sync-all` includes Copilot
- [ ] `skrills validate --target copilot` works
- [ ] Help text updated with Copilot options

---

#### TASK-011: Add MCP tools for Copilot sync

**Description**: Expose sync operations as MCP tools.

**Type**: Implementation
**Priority**: P2 (Medium)
**Estimate**: 2 points
**Dependencies**: TASK-010
**Sprint**: Sprint 2

**Linked Requirements**: FR-012

**Acceptance Criteria**:
- [ ] `sync-from-copilot` tool available
- [ ] `sync-to-copilot` tool available
- [ ] Tools return proper MCP response format
- [ ] Error handling follows MCP conventions

---

#### TASK-012: Integration tests

**Description**: End-to-end tests for Copilot sync scenarios.

**Type**: Testing
**Priority**: P1 (High)
**Estimate**: 3 points
**Dependencies**: TASK-010
**Sprint**: Sprint 2

**Acceptance Criteria**:
- [ ] Test: Claude → Copilot skill sync
- [ ] Test: Copilot → Claude skill sync
- [ ] Test: Three-way sync (Claude, Codex, Copilot)
- [ ] Test: MCP server sync preserves HTTP type
- [ ] Test: Preference sync preserves security fields
- [ ] All tests pass in CI

---

#### TASK-013: Documentation updates

**Description**: Update README and docs with Copilot support.

**Type**: Documentation
**Priority**: P2 (Medium)
**Estimate**: 2 points
**Dependencies**: TASK-010
**Sprint**: Sprint 2

**Acceptance Criteria**:
- [ ] README updated with Copilot in feature list
- [ ] Usage examples include Copilot
- [ ] Schema differences documented
- [ ] Migration guide for Copilot users

---

#### TASK-014: XDG compliance verification

**Description**: Ensure adapter respects `XDG_CONFIG_HOME` environment variable.

**Type**: Testing
**Priority**: P1 (High)
**Estimate**: 1 point
**Dependencies**: TASK-001
**Sprint**: Sprint 2

**Linked Requirements**: NFR-005

**Acceptance Criteria**:
- [ ] Test with `XDG_CONFIG_HOME` set
- [ ] Config root uses `$XDG_CONFIG_HOME/copilot`
- [ ] Falls back to `~/.copilot` when unset

---

## Dependency Graph

```
TASK-001 (Scaffold)
    ├─▶ TASK-002 (read_skills)
    │       └─▶ TASK-003 (write_skills)
    │               └─▶ TASK-008 (Discovery)
    │                       └─▶ TASK-009 (Validation)
    ├─▶ TASK-004 (read_mcp)
    │       └─▶ TASK-005 (write_mcp)
    ├─▶ TASK-006 (preferences)
    ├─▶ TASK-007 (commands no-op)
    └─▶ TASK-014 (XDG compliance)

TASK-001 through TASK-007
    └─▶ TASK-010 (CLI flags)
            ├─▶ TASK-011 (MCP tools)
            ├─▶ TASK-012 (Integration tests)
            └─▶ TASK-013 (Documentation)
```

**Critical Path**: TASK-001 → TASK-002 → TASK-003 → TASK-010 → TASK-012

**Parallel Opportunities**:
- TASK-004/005 (MCP) can run parallel to TASK-002/003 (skills)
- TASK-006 (preferences) can run parallel to skills/MCP
- TASK-007 (commands) can run anytime after TASK-001

---

## Sprint Schedule

### Sprint 1: Core Adapter

**Dates**: Week 1
**Goal**: Complete CopilotAdapter with all trait methods
**Capacity**: 16 story points

**Planned Tasks (16 points)**:
- TASK-001: CopilotAdapter scaffold (2 pts)
- TASK-002: read_skills() (3 pts)
- TASK-003: write_skills() (3 pts)
- TASK-004: read_mcp_servers() (2 pts)
- TASK-005: write_mcp_servers() (2 pts)
- TASK-006: read/write_preferences() (3 pts)
- TASK-007: commands no-op (1 pt)

**Deliverable**: `CopilotAdapter` passes all unit tests, implements full `AgentAdapter` trait

**Risks**:
- MCP config schema differences may require adjustments
- Preference field preservation logic complexity

---

### Sprint 2: Integration

**Dates**: Week 2
**Goal**: Full integration with CLI, MCP, discovery, validation
**Capacity**: 15 story points

**Planned Tasks (15 points)**:
- TASK-008: SkillSource::Copilot (2 pts)
- TASK-009: ValidationTarget::Copilot (3 pts)
- TASK-010: CLI flags (2 pts)
- TASK-011: MCP tools (2 pts)
- TASK-012: Integration tests (3 pts)
- TASK-013: Documentation (2 pts)
- TASK-014: XDG compliance (1 pt)

**Deliverable**: Full Copilot support in CLI and MCP server, passing CI

**Risks**:
- Integration complexity with existing sync orchestration
- Discovery priority conflicts with existing sources

---

## Risk Assessment

| Risk | Impact | Probability | Mitigation |
|------|--------|-------------|------------|
| MCP HTTP servers not compatible with Claude/Codex | Medium | Medium | Filter HTTP-type servers during outbound sync; document limitation |
| Security field overwrite on preferences | High | Low | Comprehensive tests; read-modify-write pattern; never touch security fields |
| Discovery priority conflicts | Medium | Low | Make priority configurable; default to same as Codex |
| Copilot CLI updates schema | Medium | Low | Version-tolerant parsing; forward compatibility |
| Performance regression in sync-all | Low | Low | Benchmark before/after; optimize if needed |

---

## Success Metrics

- [ ] All 14 tasks completed and tested
- [ ] CopilotAdapter passes same test patterns as CodexAdapter
- [ ] Bidirectional sync between Copilot, Claude, and Codex works
- [ ] No regressions in existing Claude/Codex functionality (CI green)
- [ ] Documentation updated with Copilot support
- [ ] Security fields never overwritten (test verified)

---

## Timeline

| Sprint | Week | Focus | Deliverable |
|--------|------|-------|-------------|
| 1 | 1 | Core Adapter | CopilotAdapter trait implementation |
| 2 | 2 | Integration | CLI, MCP, discovery, validation, docs |

**Total Effort**: ~31 story points across 2 sprints

---

## Next Steps

1. [ ] Review plan with stakeholders
2. [ ] Begin Sprint 1 with TASK-001
3. [ ] Use `/attune:execute` to track implementation progress
