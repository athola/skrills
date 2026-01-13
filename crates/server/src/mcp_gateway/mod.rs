//! MCP Gateway for context-optimized tool loading.
//!
//! This module implements lazy loading and context optimization for MCP tools.
//! Instead of loading all tool definitions at startup, it provides:
//!
//! - **Tool Registry**: Lightweight list of available tools without full schemas
//! - **Lazy Loading**: Load tool definitions only when explicitly requested
//! - **Context Tracking**: Track estimated token usage per tool category
//!
//! ## Design Philosophy
//!
//! Large MCP servers (like Playwright, Notion, etc.) can consume 10-20K tokens
//! just for their tool definitions. This gateway reduces context pressure by:
//!
//! 1. Exposing a lightweight tool index (name + description only)
//! 2. Loading full schemas on-demand via `describe-mcp-tool`
//! 3. Tracking which tools are "hot" for intelligent preloading
//!
//! ## MCP Tools Provided
//!
//! - `list-mcp-tools`: List available tools with minimal context cost
//! - `describe-mcp-tool`: Get full schema for a specific tool
//! - `get-context-stats`: View estimated token usage by category

mod registry;
mod stats;
mod tools;

pub use registry::{McpToolEntry, McpToolRegistry};
pub use stats::{ContextStats, ContextStatsSnapshot, ToolUsageStats};
pub use tools::{
    describe_mcp_tool, get_context_stats, list_mcp_tools, mcp_gateway_tools, MCP_GATEWAY_TOOL_NAMES,
};

/// Configuration for the MCP Gateway.
#[derive(Debug, Clone, Default)]
pub struct GatewayConfig {
    /// Whether lazy loading is enabled (always true for now).
    pub lazy_loading: bool,
    /// Maximum number of schemas to cache in memory.
    pub schema_cache_size: usize,
    /// Categories to preload at startup (empty = load none).
    pub preload_categories: Vec<String>,
}

impl GatewayConfig {
    /// Create default configuration with lazy loading enabled.
    pub fn new() -> Self {
        Self {
            lazy_loading: true,
            schema_cache_size: 100,
            preload_categories: Vec::new(),
        }
    }

    /// Configure preloading for specific categories.
    pub fn with_preload(mut self, categories: Vec<String>) -> Self {
        self.preload_categories = categories;
        self
    }
}

/// Estimate tokens for a tool definition (rough approximation).
///
/// Uses ~4 characters per token as a reasonable average for JSON schemas.
pub fn estimate_tokens(schema_json: &str) -> usize {
    schema_json.len() / 4
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        // 100 characters ≈ 25 tokens
        let schema = r#"{"type":"object","properties":{"name":{"type":"string"}}}"#;
        let tokens = estimate_tokens(schema);
        assert!(tokens > 10 && tokens < 30, "Got {tokens} tokens");
    }
}
