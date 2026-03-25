//! Tome — research API orchestration, caching, and PDF serving.
//!
//! Provides MCP server tools for unified academic/technical research:
//! - `search-papers`: Unified search across Semantic Scholar, arXiv, OpenAlex
//! - `search-discussions`: Search HN Algolia, aggregate community discussions
//! - `search-code`: GitHub code search with rate limiting
//! - `fetch-pdf`: Download PDFs by URL or DOI (via Unpaywall)
//! - `get-research-history`: Retrieve past research sessions
//! - `resolve-doi`: Resolve DOI to metadata via CrossRef

pub mod cache;
pub mod citations;
pub mod clients;
pub mod error;
pub mod knowledge_graph;
pub mod models;
pub mod rate_limit;
pub mod triz;

pub use error::{TomeError, TomeResult};
