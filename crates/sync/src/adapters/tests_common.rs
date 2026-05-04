//! Test helpers shared by adapter test suites (T3.5 bootstrap).
//!
//! The four `AgentAdapter` implementations (claude/codex/copilot/
//! cursor) historically each spelled out the same handful of basic
//! checks: `adapter.name()`, `adapter.config_root()`, an empty
//! `read_commands` directory, etc. This module centralizes those
//! checks behind small generic helpers so the adapter test files
//! converge on a single source of truth and any future check (e.g.
//! a new `FieldSupport` field) is added in one place.
//!
//! Larger parameterization (write paths, MCP server roundtrips,
//! frontmatter-strip semantics) requires per-adapter convention
//! abstraction (each adapter has different on-disk layout for
//! commands/skills/instructions) and is left as follow-up.

#![cfg(test)]

use crate::adapters::AgentAdapter;
use std::path::PathBuf;

/// Asserts the trio of adapter-basics checks that every implementor
/// honors: name string, config root reflection, and a caller-supplied
/// `FieldSupport` predicate (each adapter has its own non-trivial
/// support shape so we delegate the assertion).
pub(crate) fn assert_adapter_basics<A: AgentAdapter>(
    adapter: &A,
    expected_name: &str,
    expected_root: &PathBuf,
    field_assertions: impl FnOnce(&crate::adapters::FieldSupport),
) {
    assert_eq!(adapter.name(), expected_name);
    assert_eq!(&adapter.config_root(), expected_root);
    field_assertions(&adapter.supported_fields());
}

/// Asserts that `read_commands(false)` returns an empty list when
/// the adapter root is a fresh tempdir. Used by every implementor;
/// each adapter has a different commands-dir convention but they
/// all return empty when nothing has been written.
pub(crate) fn assert_read_commands_empty<A: AgentAdapter>(adapter: &A) {
    let commands = adapter.read_commands(false).unwrap();
    assert!(
        commands.is_empty(),
        "expected no commands on empty root, got {} entries",
        commands.len()
    );
}

/// Asserts that `read_skills()` returns an empty list on a fresh
/// tempdir root. Companion to [`assert_read_commands_empty`].
pub(crate) fn assert_read_skills_empty<A: AgentAdapter>(adapter: &A) {
    let skills = adapter.read_skills().unwrap();
    assert!(
        skills.is_empty(),
        "expected no skills on empty root, got {} entries",
        skills.len()
    );
}
