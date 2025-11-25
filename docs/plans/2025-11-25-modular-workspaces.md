# Modularize Core Into Multi-Crate Workspace Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split the current monolithic `crates/core` lib into three focused crates (`discovery`, `state`, `server`) plus the existing `cli`, keeping a single binary while enforcing clear module boundaries.

**Architecture:** Modular monolith within one workspace: `discovery` handles skill scanning/hashing; `state` owns persistence and env/config IO; `server` wires CLI + MCP server using the other crates; `cli` stays a thin entrypoint. No cross-deps between `discovery` and `state`.

**Tech Stack:** Rust 2021, Cargo workspaces, clap, tokio, notify (feature-gated), rmcp, serde.

---

### Task 1: Prepare workspace structure

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/discovery/Cargo.toml`, `crates/discovery/src/lib.rs`
- Create: `crates/state/Cargo.toml`, `crates/state/src/lib.rs`
- Create: `crates/server/Cargo.toml`, `crates/server/src/lib.rs`

**Step 1: Write the failing test**

```bash
cargo check -p codex-mcp-skills-core
```

Expected: FAIL (package not found) after removing/renaming core.

**Step 2: Run test to verify it fails**

```bash
cargo check -p codex-mcp-skills-core
```

**Step 3: Write minimal implementation**

- Update root `Cargo.toml` members to `["crates/discovery", "crates/state", "crates/server", "crates/cli"]`.
- Set new package names: `codex-mcp-skills-discovery`, `codex-mcp-skills-state`, `codex-mcp-skills-server`.
- Point `crates/cli` dependency to `codex-mcp-skills-server`.
- Add minimal `lib.rs` stubs returning `todo!()` to satisfy compiler.

Example `crates/server/src/lib.rs` stub:

```rust
pub fn run() -> anyhow::Result<()> {
    todo!("wire server once modules are moved")
}
```

**Step 4: Run test to verify it passes**

```bash
cargo check
```

Expected: PASS (with todos compiling if not executed).

**Step 5: Commit**

```bash
git add Cargo.toml crates/discovery crates/state crates/server
git commit -m "chore: scaffold discovery/state/server crates"
```

---

### Task 2: Move shared domain types into `discovery`

**Files:**
- Modify: `crates/core/src/lib.rs` (source), `crates/discovery/src/lib.rs`
- Create: `crates/discovery/src/types.rs`
- Update: `crates/server/src/lib.rs`

**Step 1: Write the failing test**

```bash
cargo check -p codex-mcp-skills-server
```

Expected: FAIL due to missing types after removal from core.

**Step 2: Run test to verify it fails**

Same command, confirm errors reference moved symbols.

**Step 3: Write minimal implementation**

- In `discovery/src/types.rs`, define `SkillSource`, `SkillRoot`, `SkillMeta`, `Diagnostics`, `DuplicateInfo` with serde derives and helper methods currently in core.
- Re-export in `discovery/src/lib.rs` with `pub use`.
- In `server/src/lib.rs`, replace old definitions with imports from `codex_mcp_skills_discovery`.

**Step 4: Run test to verify it passes**

```bash
cargo check -p codex-mcp-skills-server
```

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/discovery/src lib.rs crates/server/src/lib.rs
git commit -m "refactor: move shared types into discovery crate"
```

---

### Task 3: Extract persistence & env config into `state`

**Files:**
- Modify: `crates/core/src/lib.rs`
- Create: `crates/state/src/persistence.rs`
- Create: `crates/state/src/env.rs`
- Update: `crates/state/src/lib.rs`, `crates/server/src/lib.rs`

**Step 1: Write the failing test**

```bash
cargo check -p codex-mcp-skills-server
```

Expected: FAIL after stubbing out persistence in server.

**Step 2: Run test to verify it fails**

Same command; ensure missing functions (pinned_file, load_history, env_*).

**Step 3: Write minimal implementation**

- Move functions handling pinned/auto-pin/history/manifest IO into `persistence.rs` with structs `PinnedStore`, `HistoryStore`.
- Move env helpers (env_include_claude, env_diag, env_auto_pin, env_max_bytes, cache_ttl, extra_dirs_from_env, manifest_file) into `env.rs`.
- Re-export via `state::persistence::*` and `state::env::*`.
- Update `server` imports to use the new module paths.

**Step 4: Run test to verify it passes**

```bash
cargo check
```

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/state/src crates/server/src/lib.rs
git commit -m "refactor: split persistence and env helpers into state crate"
```

---

### Task 4: Port discovery pipeline into `discovery`

**Files:**
- Modify: `crates/core/src/lib.rs`
- Create: `crates/discovery/src/scanner.rs`
- Update: `crates/discovery/src/lib.rs`, `crates/server/src/lib.rs`

**Step 1: Write the failing test**

```bash
cargo check -p codex-mcp-skills-server
```

Expected: FAIL after removing discovery functions from server.

**Step 2: Run test to verify it fails**

Confirm errors for `discover_skills`, `load_skill_roots`, `hash_file`, `priority_labels`.

**Step 3: Write minimal implementation**

- Move filesystem walking, hashing (Sha256), AGENTS parsing (`extract_refs_from_agents`), duplicate detection, priority mapping into `scanner.rs`.
- Expose `DiscoveryConfig { roots, cache_ttl_ms, priority_override }`, `discover_skills(cfg) -> (Vec<SkillMeta>, Diagnostics)`.
- Keep watchdog feature off; no persistence here.
- Replace server calls to use `codex_mcp_skills_discovery::discover_skills`.

**Step 4: Run test to verify it passes**

```bash
cargo check
```

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/discovery/src crates/server/src/lib.rs
git commit -m "refactor: move discovery pipeline into discovery crate"
```

---

### Task 5: Rebuild `server` crate from remaining logic

**Files:**
- Modify: `crates/server/src/lib.rs`
- Remove: `crates/core/src/lib.rs`
- Update: `crates/cli/src/main.rs`

**Step 1: Write the failing test**

```bash
cargo check -p codex-mcp-skills-server
```

Expected: FAIL until run() reassembles CLI & MCP wiring.

**Step 2: Run test to verify it fails**

Same command; note undefined functions.

**Step 3: Write minimal implementation**

- Implement `run()` to parse CLI (clap derive moved here), wire subcommands to discovery/state APIs, start rmcp server runtime, SIGCHLD handler, optional `notify` watcher.
- Ensure module organization inside server (cli.rs, commands.rs if helpful) but within crate.
- Update `crates/cli/src/main.rs` to call `codex_mcp_skills_server::run()`.
- Delete `crates/core/src/lib.rs` and remove crate entry from workspace.

**Step 4: Run test to verify it passes**

```bash
cargo check
cargo test
```

Expected: PASS (tests may be limited to doctests).

**Step 5: Commit**

```bash
git add crates/server crates/cli Cargo.toml
git rm crates/core/src/lib.rs
git commit -m "feat: assemble server crate and remove monolithic core"
```

---

### Task 6: Add targeted tests and docs

**Files:**
- Create: `crates/discovery/tests/discover_smoke.rs`
- Create: `crates/state/tests/persistence_smoke.rs`
- Modify: `README.md` (architecture section)
- Modify: `docs/CHANGELOG.md`

**Step 1: Write the failing test**

```bash
cargo test -p codex-mcp-skills-discovery -p codex-mcp-skills-state
```

Expected: FAIL (tests not yet implemented).

**Step 2: Run test to verify it fails**

Same command; see missing files.

**Step 3: Write minimal implementation**

- Add smoke test that builds temporary dir with fake SKILL.md and asserts discovery returns it.
- Add persistence test creating temp dir for pinned/history JSON round-trip.
- Document new crate layout in README and changelog.

**Step 4: Run test to verify it passes**

```bash
cargo test
```

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/discovery/tests crates/state/tests README.md docs/CHANGELOG.md
git commit -m "test/docs: add smoke tests and document modular crates"
```

---

### Task 7: Final verification

**Files:**
- none (commands only)

**Step 1: Write the failing test**

```bash
make test --quiet
```

Expected: PASS (should already pass; treat as verification).

**Step 2: Run test to verify it fails**

If failure occurs, fix before proceeding.

**Step 3: Write minimal implementation**

N/A (verification).

**Step 4: Run test to verify it passes**

Repeat `make test --quiet` until green.

**Step 5: Commit**

```bash
git add .
git commit -m "chore: finalize modular workspace refactor"
```

---

### Task 8: Add coverage workflow and badges

**Files:**
- Create: `.github/workflows/coverage.yml`
- Modify: `README.md`

**Step 1: Write the failing test**

```bash
rg "coverage.yml" .github/workflows || true
```

Expected: No file found.

**Step 2: Run test to verify it fails**

Same command, confirm absence.

**Step 3: Write minimal implementation**

- Add GitHub Actions workflow running `cargo llvm-cov --workspace --lcov --output-path lcov.info`, upload artifact, and send to Codecov.
- Add badges (CI, Coverage workflow status, Codecov, Audit, Docs) near README header.
- Add local coverage instructions under Development.

**Step 4: Run test to verify it passes**

```bash
cargo llvm-cov --workspace --no-report --fail-under-lines 0
```

Expected: PASS.

**Step 5: Commit**

```bash
git add .github/workflows/coverage.yml README.md
git commit -m "ci: add coverage workflow and badges"
```

---

Plan complete and saved to `docs/plans/2025-11-25-modular-workspaces.md`. Two execution options:

1. Subagent-Driven (this session) — dispatch fresh subagent per task with reviews between tasks.
2. Parallel Session — open a new session and use superpowers:executing-plans to follow the plan.

Which approach?
