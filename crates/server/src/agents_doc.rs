//! The `doc://agents` MCP resource. Its text is the default AGENTS.md that
//! `skrills_sync::sync_agents` writes, so both read [`skrills_sync::AGENTS_TEXT`].

/// URI for the AGENTS.md document.
pub(crate) const AGENTS_URI: &str = "doc://agents";
/// Name of the AGENTS.md document.
pub(crate) const AGENTS_NAME: &str = "AGENTS.md";
/// Description of the AGENTS.md document.
pub(crate) const AGENTS_DESCRIPTION: &str = "AI Agent Development Guidelines";
/// Environment variable to control exposure of AGENTS.md.
pub(crate) const ENV_EXPOSE_AGENTS: &str = "SKRILLS_EXPOSE_AGENTS";
