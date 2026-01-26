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

mod config;
mod dependency;
mod intelligence;
mod mcp;
mod resource;
mod search;
mod trace;
mod validation;
