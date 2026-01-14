//! Tool registry for lightweight MCP tool tracking.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A lightweight entry for an MCP tool (no full schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolEntry {
    /// Tool name (e.g., "browser_snapshot").
    pub name: String,
    /// Brief description of what the tool does.
    pub description: String,
    /// Source server/plugin providing this tool.
    pub source: String,
    /// Estimated token cost if full schema is loaded.
    pub estimated_tokens: usize,
    /// Category for grouping (e.g., "browser", "notion", "sentry").
    pub category: Option<String>,
}

/// Registry of available MCP tools with minimal metadata.
#[derive(Debug, Clone, Default)]
pub struct McpToolRegistry {
    /// Tools indexed by name.
    tools: HashMap<String, McpToolEntry>,
    /// Tools grouped by source server.
    by_source: HashMap<String, Vec<String>>,
    /// Tools grouped by category.
    by_category: HashMap<String, Vec<String>>,
}

impl McpToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a tool entry.
    pub fn register(&mut self, entry: McpToolEntry) {
        let name = entry.name.clone();
        let source = entry.source.clone();
        let category = entry.category.clone();

        self.by_source.entry(source).or_default().push(name.clone());

        if let Some(cat) = category {
            self.by_category.entry(cat).or_default().push(name.clone());
        }

        self.tools.insert(name, entry);
    }

    /// Get a tool entry by name.
    pub fn get(&self, name: &str) -> Option<&McpToolEntry> {
        self.tools.get(name)
    }

    /// List all tool entries.
    pub fn list_all(&self) -> Vec<&McpToolEntry> {
        self.tools.values().collect()
    }

    /// List tools from a specific source.
    pub fn list_by_source(&self, source: &str) -> Vec<&McpToolEntry> {
        self.by_source
            .get(source)
            .map(|names| names.iter().filter_map(|n| self.tools.get(n)).collect())
            .unwrap_or_default()
    }

    /// List tools in a specific category.
    pub fn list_by_category(&self, category: &str) -> Vec<&McpToolEntry> {
        self.by_category
            .get(category)
            .map(|names| names.iter().filter_map(|n| self.tools.get(n)).collect())
            .unwrap_or_default()
    }

    /// Get all unique sources.
    pub fn sources(&self) -> Vec<&str> {
        self.by_source.keys().map(String::as_str).collect()
    }

    /// Get all unique categories.
    pub fn categories(&self) -> Vec<&str> {
        self.by_category.keys().map(String::as_str).collect()
    }

    /// Total number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Calculate total estimated tokens if all tools were loaded.
    pub fn total_estimated_tokens(&self) -> usize {
        self.tools.values().map(|e| e.estimated_tokens).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_operations() {
        let mut registry = McpToolRegistry::new();

        registry.register(McpToolEntry {
            name: "browser_snapshot".into(),
            description: "Take a browser snapshot".into(),
            source: "playwright".into(),
            estimated_tokens: 150,
            category: Some("browser".into()),
        });

        registry.register(McpToolEntry {
            name: "browser_click".into(),
            description: "Click an element".into(),
            source: "playwright".into(),
            estimated_tokens: 200,
            category: Some("browser".into()),
        });

        registry.register(McpToolEntry {
            name: "notion_search".into(),
            description: "Search Notion".into(),
            source: "notion".into(),
            estimated_tokens: 300,
            category: Some("database".into()),
        });

        assert_eq!(registry.len(), 3);
        assert_eq!(registry.list_by_source("playwright").len(), 2);
        assert_eq!(registry.list_by_category("browser").len(), 2);
        assert_eq!(registry.total_estimated_tokens(), 650);
    }
}
