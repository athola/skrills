//! Tests for the app module.
//!
//! This module contains tests organized into logical submodules:
//!
//! - `config` - Skill root selection, project directory resolution
//! - `validation` - Skill validation, autofix, dependency checking
//! - `dependency` - Dependency graph, transitive resolution
//! - `resource` - Resource reading with dependency resolution
//! - `search` - Fuzzy skill search
//! - `trace` - Skill loading status, trace instrumentation
//! - `intelligence` - Smart recommendations, skill creation, GitHub search
//! - `mcp` - MCP registry, tool operations, context stats
//! - `sync` - Sync error paths and parameter handling
//! - `research` - Knowledge graph, citation tracking, TRIZ resolution

mod config;
mod dependency;
mod intelligence;
mod mcp;
mod research;
mod resource;
mod search;
mod sync;
mod trace;
mod validation;
