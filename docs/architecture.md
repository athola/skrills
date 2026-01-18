# Architecture

The CLI delegates command parsing to focused handlers that call subsystems for discovery, synchronization, and runtime management. The `app` module connects the CLI to the `SkillService`, which exposes resources and tools over MCP.

## Runtime Flow

```mermaid
graph TD
    CLI[CLI parser] --> Commands[commands/* handlers]
    Commands --> App[app::run]
    App --> SkillService
    SkillService --> Discovery[discovery & validation]
    SkillService --> State[manifest & cache TTL]
    App --> Sync[sync + mirror]
    App --> Setup[setup]
    App --> Tui[tui flow]
    App --> Doctor[doctor diagnostics]
```

## Crate Dependency Graph

```mermaid
graph TD
    cli[cli] --> server[server<br/>MCP runtime, commands]
    server --> sync[sync]
    server --> validate[validate]
    server --> analyze[analyze]
    server --> intelligence[intelligence]
    server --> subagents[subagents]
    sync --> validate
    analyze --> validate
    analyze --> discovery
    subagents --> state[state]

    subgraph leaf["Leaf Crates (no internal deps)"]
        discovery[discovery]
        intelligence
        state
        validate
    end
```

## Crate Responsibilities

| Crate | Purpose |
|-------|---------|
| `cli` | Thin binary wrapper |
| `server` | MCP server, CLI commands, TUI |
| `sync` | Bidirectional Claude/Codex/Copilot sync |
| `validate` | Skill validation (Claude/Codex/Copilot) |
| `analyze` | Token counting, dependencies |
| `intelligence` | Recommendations, project analysis, skill generation |
| `discovery` | Skill/agent discovery, ranking |
| `state` | Environment config, persistence |
| `subagents` | Multi-backend agent runtime |

## Design Principles

The architecture separates concerns strictly. Leaf crates like `validate`, `discovery`, and `state` have no internal dependencies, while near-leaf crates like `analyze` only depend on those specific types. We use trait-based abstraction via `AgentAdapter` to support pluggable source and target adapters for Claude, Codex, and Copilot, and compile-time dispatch through `SyncOrchestrator<S, T>` to maintain performance and type safety. Features like `subagents` and `watch` are gated behind feature flags to keep the core binary small.

## Module Organization

The `app` module is split to stay under the 2500 LOC threshold (ADR-0001):

| Module | Lines | Purpose |
|--------|-------|---------|
| `mod.rs` | ~1600 | Core SkillService, MCP handlers, resource serving |
| `intelligence.rs` | ~740 | Intelligence tool implementations |

**LOC Monitoring**: When `app/mod.rs` approaches 2000 lines, extract the next logical group (e.g., subagent tool handlers or resource-serving methods).

## Roadmap

Future development will focus on extracting command handlers to submodules as the CLI expands and aligning the CLI and MCP tool lists. We plan to evaluate consolidating `sync-from-claude` with `sync-all` and versioning intelligence tool inputs/outputs. Architectural changes will continue to be documented in ADRs, and we will monitor the `app` module size to trigger further refactoring when needed.

## Related Documents

- [ADR 0001: Pivot to Support Engine](adr/0001-pivot-to-support-engine.md)
- [ADR 0002: Skill Dependency Resolution](adr/0002-skill-dependency-resolution.md)
- [ADR 0003: CLI Parity for Intelligence Tools](adr/0003-cli-parity-intelligence-tools.md)
- [ADR 0004: Intelligence Crate Versioning](adr/0004-intelligence-crate-versioning.md)
- [Book: Overview](../book/src/overview.md)
