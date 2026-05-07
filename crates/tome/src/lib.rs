//! Tome — research API orchestration, caching, and knowledge graph.
//!
//! Provides clients for academic and technical research:
//! - Semantic Scholar, arXiv, OpenAlex paper search
//! - Hacker News (Algolia) discussion search
//! - CrossRef DOI resolution
//! - Unpaywall open-access PDF lookup
//! - SQLite-backed caching with TTL
//! - Persistent knowledge graph for cross-session research
//! - Citation tracking for forward/backward traversal
//! - TRIZ inventive principles adapted for software

pub mod cache;
pub mod citations;
pub mod clients;
pub mod dispatcher;
pub mod error;
pub mod knowledge_graph;
pub mod models;
pub mod triz;

pub use error::{TomeError, TomeResult};
