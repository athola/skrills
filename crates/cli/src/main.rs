//! Command-line interface for the `codex-mcp-skills` application.
//!
//! This crate serves as the main entry point for the executable, delegating
//! its core functionality to the `codex-mcp-skills-core` crate.

fn main() -> anyhow::Result<()> {
    codex_mcp_skills_server::run()
}
