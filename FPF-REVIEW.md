# FPF Architecture Review: skrills

**Review Date:** 2026-02-01
**Codebase:** skrills (Skill Registry & Management System)
**Methodology:** Features-Practical-Foundation (FPF) Analysis

---

## Executive Summary

skrills is a well-architected skill management system comprising 12 crates (~27,346 LOC) with strong SOLID adherence and comprehensive test coverage (1,883 tests). The architecture employs appropriate design patterns (Adapter, Plugin, Orchestrator, Strategy) for multi-agent support across Claude, Codex, and Copilot. **Critical performance issue identified:** the `/api/skills` endpoint rescans the filesystem on every request without caching, which will cause latency spikes under load.

---

## Feature Inventory

| Feature | Status | Notes |
|---------|--------|-------|
| Skill Validation | Complete | YAML/TOML/JSON schema validation |
| Token Analysis | Complete | Context window optimization |
| Multi-directional Sync | Complete | Claude/Codex/Copilot adapters |
| MCP Server | Complete | 40+ tools exposed |
| Dashboard (TUI) | Complete | Real-time metrics display |
| Dashboard (Browser) | Complete | Leptos SSR implementation |
| Dependency Resolution | Complete | DAG-based ordering |
| Metrics Collection | Complete | SQLite WAL, 30-day retention |
| Runtime Injection | Not Implemented | By design - static configuration |
| Copilot Commands | Disabled | Adapter present but inactive |
| LLM Intelligence | Partial | Requires API key configuration |

---

## Performance Assessment

| Metric | Current | Status |
|--------|---------|--------|
| Skill Discovery Caching | None | CRITICAL - rescans every request |
| Pagination (skill list) | Missing | WARNING - unbounded responses |
| Database Mode | SQLite WAL | OK - concurrent reads |
| Test Coverage | 1,883 tests | EXCELLENT |
| Data Retention | 30 days | OK - configurable |
| Response Time (cached) | N/A | Not measured - no caching |

---

## Pattern Analysis

| Pattern | Where Used | Assessment |
|---------|------------|------------|
| Adapter | `subagents/` - Claude, Codex, Copilot | Appropriate - isolates LLM-specific logic |
| Plugin | `discovery/` - file type scanners | Appropriate - extensible format support |
| Orchestrator | `sync/` - multi-target coordination | Appropriate - manages complex workflows |
| Strategy | `validate/` - validation rules | Appropriate - swappable validation logic |
| Repository | `state/` - skill persistence | Appropriate - data access abstraction |

---

## Technical Debt

| Item | Severity | Effort | Priority |
|------|----------|--------|----------|
| No caching on `/api/skills` | Critical | Medium | P0 |
| Missing pagination on skill list | High | Low | P1 |
| Debug format bug in `SkillSource` | Medium | Low | P1 |
| Copilot adapter complexity (5+ modules) | Medium | High | P2 |
| `SyncParams` field explosion (10+ fields) | Medium | Medium | P2 |
| No request ID logging | Low | Low | P3 |
| SQLite for time-series metrics | Low | High | P3 |
| Missing adapter contract documentation | Low | Low | P3 |

---

## Prioritized Recommendations

### High Priority (P0-P1)

1. **Implement Skill Discovery Caching**
   - Add mtime-based cache invalidation
   - Set TTL of 30-60 seconds for scan results
   - Consider file watcher for immediate invalidation
   - Impact: Eliminates redundant filesystem scans

2. **Add Pagination to `/api/skills`**
   - Implement cursor-based pagination
   - Default page size: 50, max: 200
   - Include total count in response metadata
   - Impact: Prevents unbounded memory usage

3. **Fix `SkillSource` Debug Output Bug**
   - Location: `format!("{:?}", meta.source)` produces raw debug output
   - Replace with proper `Display` implementation
   - Impact: Improves log readability

### Medium Priority (P2)

4. **Consolidate Copilot Adapter**
   - Current: 5+ sub-modules with overlapping concerns
   - Target: Single cohesive module with clear boundaries
   - Consider feature flags for optional components
   - Impact: Reduces maintenance burden

5. **Refactor `SyncParams` with Builder Pattern**
   - Current: 10+ fields causing constructor explosion
   - Implement `SyncParamsBuilder` with sensible defaults
   - Add validation in `build()` method
   - Impact: Improves API ergonomics

6. **Add Request ID Logging**
   - Generate UUID for each API request
   - Propagate through all log statements
   - Include in error responses
   - Impact: Enables request tracing

### Low Priority (P3)

7. **Evaluate Time-Series Database for Metrics**
   - Current SQLite adequate for current scale
   - Consider InfluxDB/TimescaleDB if metrics volume grows
   - Impact: Future scalability (not urgent)

8. **Document Adapter Contracts**
   - Add interface documentation for subagent adapters
   - Include example implementations
   - Impact: Eases third-party adapter development

---

## Action Items

- [ ] **P0:** Implement caching layer for skill discovery (mtime + TTL)
- [ ] **P1:** Add pagination to `/api/skills` endpoint
- [ ] **P1:** Fix `SkillSource` Debug format bug
- [ ] **P2:** Audit Copilot adapter for consolidation opportunities
- [ ] **P2:** Create `SyncParamsBuilder` with defaults
- [ ] **P2:** Add request ID middleware to server
- [ ] **P3:** Document adapter interface contracts
- [ ] **P3:** Benchmark metrics storage for scaling assessment

---

## Appendix: Crate Summary

| Crate | LOC (approx) | Purpose |
|-------|--------------|---------|
| server | 4,500 | HTTP API, MCP server |
| cli | 3,200 | Command-line interface (39 commands) |
| validate | 2,800 | Schema validation |
| analyze | 2,400 | Token analysis |
| discovery | 2,100 | Skill scanning |
| intelligence | 1,900 | LLM integration |
| sync | 2,600 | Multi-target sync |
| subagents | 3,100 | Claude/Codex/Copilot adapters |
| metrics | 1,800 | Telemetry collection |
| dashboard | 1,500 | TUI + Leptos web UI |
| state | 1,100 | Persistence layer |
| test-utils | 346 | Test helpers |

---

*Report generated via FPF methodology - Features, Practical, Foundation analysis.*
