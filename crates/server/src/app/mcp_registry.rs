//! MCP tool registry construction and tool-name categorization.
//!
//! Split out of `app/mod.rs` (T3.1 of the v0.8.0 refinement plan).
//! Houses the [`build_mcp_registry`] entrypoint that the
//! [`super::SkillService`] constructor calls once at startup, plus
//! the [`ToolCategory`] enum used to tag tool entries by purpose.
//! Both are pure construction-time concerns; isolating them keeps
//! `SkillService::new_with_ttl` focused on lifecycle wiring.

use crate::mcp_gateway::{McpToolEntry, McpToolRegistry};
use crate::tool_schemas;

/// Categorizes tools by their primary purpose.
///
/// Used for organizing and filtering tools by their primary purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolCategory {
    Sync,
    Validation,
    Trace,
    Intelligence,
    Metrics,
    Dependency,
    Gateway,
}

impl ToolCategory {
    /// Infer category from a tool name using prefix/substring matching.
    pub(crate) fn from_tool_name(name: &str) -> Option<Self> {
        match name {
            n if n.starts_with("sync") => Some(Self::Sync),
            n if n.starts_with("validate") || n.starts_with("analyze") => Some(Self::Validation),
            n if n.contains("trace") || n.contains("instrument") => Some(Self::Trace),
            n if n.contains("recommend") || n.contains("suggest") => Some(Self::Intelligence),
            n if n.contains("metric") => Some(Self::Metrics),
            n if n.contains("depend") => Some(Self::Dependency),
            _ => None,
        }
    }

    /// Convert to a string representation for serialization.
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Validation => "validation",
            Self::Trace => "trace",
            Self::Intelligence => "intelligence",
            Self::Metrics => "metrics",
            Self::Dependency => "dependency",
            Self::Gateway => "gateway",
        }
    }
}

/// Builds the MCP tool registry from available definitions.
pub(crate) fn build_mcp_registry() -> McpToolRegistry {
    use crate::mcp_gateway::estimate_tokens;

    let mut registry = McpToolRegistry::new();

    // Register all internal tools from tool_schemas
    for tool in tool_schemas::all_tools() {
        let schema_json = serde_json::to_string(&tool.input_schema).unwrap_or_default();
        let estimated_tokens = estimate_tokens(&schema_json);

        // Infer category from tool name using enum matching
        let category = ToolCategory::from_tool_name(&tool.name).map(|c| c.as_str().to_string());

        registry.register(McpToolEntry {
            name: tool.name.to_string(),
            description: tool.description.clone().unwrap_or_default().to_string(),
            source: "skrills".to_string(),
            estimated_tokens,
            category,
        });
    }

    // Register gateway tools themselves
    for tool in crate::mcp_gateway::mcp_gateway_tools() {
        let schema_json = serde_json::to_string(&tool.input_schema).unwrap_or_default();
        let estimated_tokens = estimate_tokens(&schema_json);
        registry.register(McpToolEntry {
            name: tool.name.to_string(),
            description: tool.description.clone().unwrap_or_default().to_string(),
            source: "gateway".to_string(),
            estimated_tokens,
            category: Some(ToolCategory::Gateway.as_str().to_string()),
        });
    }

    registry
}
