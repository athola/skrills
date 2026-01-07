# Issues Execution Plan

## Overview
Total Issues: 14
- Testing: 10 issues (#55-61)
- Bug Fixes: 3 issues (#42, #44, #45)
- Enhancements: 2 issues (#47, #48)
- Documentation: 1 issue (#39)
- Feature Request: 1 issue (#49)

## Task Breakdown

### Batch 1: Independent Tasks (Can run in parallel)

#### Testing Tasks (Independent modules)
1. **Issue #55**: test(intelligence) - Server intelligence tools
   - File: `crates/server/src/app/intelligence.rs`
   - Coverage: 36.75% (363 lines missing)
   - Effort: Medium

2. **Issue #56**: test(usage) - Usage parsers
   - Files: `crates/intelligence/src/usage/claude_parser.rs`, `codex_parser.rs`
   - Coverage: 37.07% and 49.75%
   - Effort: Medium

3. **Issue #57**: test(create) - Skill creation module
   - File: `crates/intelligence/src/create/mod.rs`
   - Coverage: 0% (63 lines missing)
   - Effort: Low

4. **Issue #58**: test(github-search) - GitHub search integration
   - File: `crates/intelligence/src/create/github_search.rs`
   - Coverage: 62.72% (82 lines missing)
   - Effort: Medium

5. **Issue #59**: test(llm-generator) - LLM skill generation
   - File: `crates/intelligence/src/create/llm_generator.rs`
   - Coverage: 69.26% (63 lines missing)
   - Effort: Medium

6. **Issue #60**: test(context) - Project context detection
   - Files: `crates/intelligence/src/context/detector.rs`, `dependencies.rs`
   - Coverage: 84.92% and 74.03%
   - Effort: Medium

7. **Issue #61**: test(recommend) - Recommendation scorer
   - File: `crates/intelligence/src/recommend/scorer.rs`
   - Coverage: 61.17% (66 lines missing)
   - Effort: Low

#### Bug Fixes (Independent)
8. **Issue #42**: Enhancement - Warn on skipped optional dependencies
   - File: `crates/analyze/src/resolve.rs:592`
   - Effort: Low

9. **Issue #44**: Enhancement - Add line/column to YAML errors
   - File: `crates/validate/src/frontmatter.rs:243`
   - Effort: Low

10. **Issue #45**: Bug - Handle invalid regex gracefully
    - Files: `crates/analyze/src/deps.rs:124-128`
    - Effort: Low (HIGH priority - prevents panics)

#### Enhancements (Independent)
11. **Issue #47**: Enhancement - Actionable hints for file warnings
    - File: `crates/server/src/skill_trace.rs:303`
    - Effort: Low

12. **Issue #48**: Enhancement - Log unreadable metadata files
    - File: `crates/analyze/src/deps.rs:92-94`
    - Effort: Low

#### Documentation (Independent)
13. **Issue #39**: Documentation - Create audit-logging.md
    - File: `docs/audit-logging.md` (new)
    - Effort: Medium

### Batch 2: Sequential/Complex Tasks

14. **Issue #49**: Feature Request - MCP context optimization
    - Complex multi-phase feature requiring:
      - Phase 1: MCP Gateway Service
      - Phase 2: Context Optimization Engine
      - Phase 3: Advanced Features
    - Effort: Very High (8-10 weeks)
    - Status: **DEFER** - This is a major feature requiring dedicated implementation

### Additional Issue

15. **Issue #53**: Chore - Rename skrills_sync to skrills-sync
    - Breaking change requiring workspace-wide updates
    - Status: **DEFER** - Should be done during major version bump

## Execution Strategy

### Immediate Execution (Batch 1)
Execute issues #39, #42, #44, #45, #47, #48, #55-61 in parallel using subagents.

### Deferred (Batch 2)
- Issue #49: Too complex for batch fixing, requires dedicated implementation
- Issue #53: Breaking change, defer to major version bump

## Dependency Analysis

**No dependencies between Batch 1 tasks** - all can be executed independently:
- Testing tasks target different modules
- Bug fixes target different files
- Enhancements target different files
- Documentation is standalone

## Success Criteria

Each issue should:
- [ ] Pass all existing tests
- [ ] Add new tests with >80% coverage for target files
- [ ] Follow project coding standards
- [ ] Include documentation updates where applicable
- [ ] Pass linting (ruff, clippy)
