//! Persistent knowledge graph for cross-session research accumulation.
//!
//! Nodes: topics, papers, implementations, discussions
//! Edges: cites, implements, contradicts, extends, analogous_to

use crate::{TomeError, TomeResult};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;
use time::OffsetDateTime;

/// Node types in the knowledge graph.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, strum::EnumIter, strum::EnumCount,
)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Topic,
    Paper,
    Implementation,
    Discussion,
}

impl NodeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Paper => "paper",
            Self::Implementation => "implementation",
            Self::Discussion => "discussion",
        }
    }

    /// Returns all variants in declaration order.
    pub fn all() -> &'static [NodeKind] {
        static ALL: LazyLock<Vec<NodeKind>> =
            LazyLock::new(|| <NodeKind as strum::IntoEnumIterator>::iter().collect());
        &ALL
    }
}

/// Edge types (relationships) in the knowledge graph.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, strum::EnumIter, strum::EnumCount,
)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Cites,
    Implements,
    Contradicts,
    Extends,
    AnalogousTo,
}

impl EdgeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cites => "cites",
            Self::Implements => "implements",
            Self::Contradicts => "contradicts",
            Self::Extends => "extends",
            Self::AnalogousTo => "analogous_to",
        }
    }

    /// Returns all variants in declaration order.
    pub fn all() -> &'static [EdgeKind] {
        static ALL: LazyLock<Vec<EdgeKind>> =
            LazyLock::new(|| <EdgeKind as strum::IntoEnumIterator>::iter().collect());
        &ALL
    }
}

/// A node in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    pub metadata_json: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// An edge between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub source_id: String,
    pub target_id: String,
    pub kind: EdgeKind,
    pub weight: f64,
    pub metadata_json: Option<String>,
}

/// SQLite-backed knowledge graph.
pub struct KnowledgeGraph {
    conn: std::sync::Mutex<Connection>,
}

impl KnowledgeGraph {
    /// Opens or creates the knowledge graph database.
    pub fn open(db_path: &std::path::Path) -> TomeResult<Self> {
        let conn = Connection::open(db_path)?;
        let kg = Self {
            conn: std::sync::Mutex::new(conn),
        };
        kg.init_schema()?;
        Ok(kg)
    }

    /// Opens an in-memory graph (for testing).
    pub fn open_in_memory() -> TomeResult<Self> {
        let conn = Connection::open_in_memory()?;
        let kg = Self {
            conn: std::sync::Mutex::new(conn),
        };
        kg.init_schema()?;
        Ok(kg)
    }

    // NOTE: CREATE TABLE IF NOT EXISTS means FK constraints only apply to new
    // databases. Pre-existing databases keep the old schema without FKs.
    // Acceptable pre-1.0; a migration step should be added before 1.0 release.
    fn init_schema(&self) -> TomeResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS nodes (
                id TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                label TEXT NOT NULL,
                metadata_json TEXT,
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS edges (
                source_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                weight REAL NOT NULL DEFAULT 1.0,
                metadata_json TEXT,
                PRIMARY KEY (source_id, target_id, kind),
                FOREIGN KEY (source_id) REFERENCES nodes(id),
                FOREIGN KEY (target_id) REFERENCES nodes(id)
            );

            CREATE INDEX IF NOT EXISTS idx_nodes_kind ON nodes(kind);
            CREATE INDEX IF NOT EXISTS idx_nodes_label ON nodes(label);
            CREATE INDEX IF NOT EXISTS idx_edges_source ON edges(source_id);
            CREATE INDEX IF NOT EXISTS idx_edges_target ON edges(target_id);
            CREATE INDEX IF NOT EXISTS idx_edges_kind ON edges(kind);",
        )?;
        Ok(())
    }

    /// Add a node to the graph.
    pub fn add_node(
        &self,
        id: &str,
        kind: NodeKind,
        label: &str,
        metadata_json: Option<&str>,
    ) -> TomeResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let now = OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| TomeError::Other(format!("time format error: {e}")))?;
        conn.execute(
            "INSERT INTO nodes (id, kind, label, metadata_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, label=excluded.label, metadata_json=excluded.metadata_json",
            rusqlite::params![id, kind.as_str(), label, metadata_json, now],
        )?;
        Ok(())
    }

    /// Add an edge between two nodes.
    pub fn add_edge(
        &self,
        source_id: &str,
        target_id: &str,
        kind: EdgeKind,
        weight: f64,
        metadata_json: Option<&str>,
    ) -> TomeResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO edges (source_id, target_id, kind, weight, metadata_json) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(source_id, target_id, kind) DO UPDATE SET weight=excluded.weight, metadata_json=excluded.metadata_json",
            rusqlite::params![source_id, target_id, kind.as_str(), weight, metadata_json],
        )?;
        Ok(())
    }

    /// Get a node by ID.
    pub fn get_node(&self, id: &str) -> TomeResult<Option<Node>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let result = conn.query_row(
            "SELECT id, kind, label, metadata_json, created_at FROM nodes WHERE id = ?1",
            [id],
            |row| {
                Ok(Node {
                    id: row.get(0)?,
                    kind: parse_node_kind(row.get::<_, String>(1)?.as_str())
                        .map_err(|e| rusqlite::Error::InvalidColumnName(format!("{e}")))?,
                    label: row.get(2)?,
                    metadata_json: row.get(3)?,
                    created_at: parse_timestamp(row.get::<_, String>(4)?.as_str())
                        .map_err(|e| rusqlite::Error::InvalidColumnName(format!("{e}")))?,
                })
            },
        );
        match result {
            Ok(node) => Ok(Some(node)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(TomeError::Cache(e)),
        }
    }

    /// Query: "what do I know about X?" -- find nodes matching a label pattern.
    pub fn search_nodes(&self, query: &str, kind: Option<NodeKind>) -> TomeResult<Vec<Node>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let pattern = format!("%{query}%");
        let sql = match kind {
            Some(k) => {
                let mut stmt = conn.prepare(
                    "SELECT id, kind, label, metadata_json, created_at FROM nodes WHERE label LIKE ?1 AND kind = ?2 ORDER BY created_at DESC"
                )?;
                let rows = stmt.query_map(rusqlite::params![pattern, k.as_str()], |row| {
                    Ok(Node {
                        id: row.get(0)?,
                        kind: parse_node_kind(row.get::<_, String>(1)?.as_str())
                        .map_err(|e| rusqlite::Error::InvalidColumnName(format!("{e}")))?,
                        label: row.get(2)?,
                        metadata_json: row.get(3)?,
                        created_at: parse_timestamp(row.get::<_, String>(4)?.as_str())
                            .map_err(|e| rusqlite::Error::InvalidColumnName(format!("{e}")))?,
                    })
                })?;
                return rows.collect::<Result<Vec<_>, _>>().map_err(TomeError::Cache);
            }
            None => "SELECT id, kind, label, metadata_json, created_at FROM nodes WHERE label LIKE ?1 ORDER BY created_at DESC",
        };
        let mut stmt = conn.prepare(sql)?;
        let rows = stmt.query_map([&pattern], |row| {
            Ok(Node {
                id: row.get(0)?,
                kind: parse_node_kind(row.get::<_, String>(1)?.as_str())
                    .map_err(|e| rusqlite::Error::InvalidColumnName(format!("{e}")))?,
                label: row.get(2)?,
                metadata_json: row.get(3)?,
                created_at: parse_timestamp(row.get::<_, String>(4)?.as_str())
                    .map_err(|e| rusqlite::Error::InvalidColumnName(format!("{e}")))?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(TomeError::Cache)
    }

    /// Find all edges from a node (outgoing connections).
    pub fn edges_from(&self, node_id: &str) -> TomeResult<Vec<Edge>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT source_id, target_id, kind, weight, metadata_json FROM edges WHERE source_id = ?1"
        )?;
        let rows = stmt.query_map([node_id], |row| {
            Ok(Edge {
                source_id: row.get(0)?,
                target_id: row.get(1)?,
                kind: parse_edge_kind(row.get::<_, String>(2)?.as_str())
                    .map_err(|e| rusqlite::Error::InvalidColumnName(format!("{e}")))?,
                weight: row.get(3)?,
                metadata_json: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(TomeError::Cache)
    }

    /// Find all edges to a node (incoming connections).
    pub fn edges_to(&self, node_id: &str) -> TomeResult<Vec<Edge>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT source_id, target_id, kind, weight, metadata_json FROM edges WHERE target_id = ?1"
        )?;
        let rows = stmt.query_map([node_id], |row| {
            Ok(Edge {
                source_id: row.get(0)?,
                target_id: row.get(1)?,
                kind: parse_edge_kind(row.get::<_, String>(2)?.as_str())
                    .map_err(|e| rusqlite::Error::InvalidColumnName(format!("{e}")))?,
                weight: row.get(3)?,
                metadata_json: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(TomeError::Cache)
    }

    /// Count total nodes and edges.
    pub fn stats(&self) -> TomeResult<(usize, usize)> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let nodes: i64 = conn.query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))?;
        let edges: i64 = conn.query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))?;
        Ok((nodes as usize, edges as usize))
    }
}

/// Legacy SQLite `datetime('now')` format: `YYYY-MM-DD HH:MM:SS`
static LEGACY_TIMESTAMP_FMT: LazyLock<Vec<time::format_description::FormatItem<'static>>> =
    LazyLock::new(|| {
        time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second]")
            .expect("static format description")
    });

/// Parse a timestamp string from SQLite TEXT storage into `OffsetDateTime`.
///
/// Accepts RFC 3339 (`2026-04-08T12:00:00Z`) and falls back to SQLite's
/// `datetime('now')` format (`2026-04-08 12:00:00`) for databases created
/// before the 0.7.5 migration.  Fallback timestamps are treated as UTC.
fn parse_timestamp(s: &str) -> TomeResult<OffsetDateTime> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
        .or_else(|_| {
            tracing::debug!("Parsing legacy timestamp format: '{s}'");
            time::PrimitiveDateTime::parse(s, &LEGACY_TIMESTAMP_FMT).map(|dt| dt.assume_utc())
        })
        .map_err(|e| TomeError::Other(format!("invalid timestamp '{s}': {e}")))
}

fn parse_node_kind(s: &str) -> TomeResult<NodeKind> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|_| TomeError::Other(format!("unknown NodeKind: {s}")))
}

fn parse_edge_kind(s: &str) -> TomeResult<EdgeKind> {
    serde_json::from_value(serde_json::Value::String(s.to_owned()))
        .map_err(|_| TomeError::Other(format!("unknown EdgeKind: {s}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_retrieve_node() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node(
            "paper-1",
            NodeKind::Paper,
            "Attention Is All You Need",
            None,
        )
        .unwrap();
        let node = kg.get_node("paper-1").unwrap().unwrap();
        assert_eq!(node.label, "Attention Is All You Need");
        assert_eq!(node.kind, NodeKind::Paper);
    }

    #[test]
    fn add_and_query_edges() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node("paper-1", NodeKind::Paper, "Transformer Paper", None)
            .unwrap();
        kg.add_node("paper-2", NodeKind::Paper, "BERT Paper", None)
            .unwrap();
        kg.add_edge("paper-2", "paper-1", EdgeKind::Cites, 1.0, None)
            .unwrap();

        let edges = kg.edges_from("paper-2").unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].target_id, "paper-1");
        assert_eq!(edges[0].kind, EdgeKind::Cites);

        let incoming = kg.edges_to("paper-1").unwrap();
        assert_eq!(incoming.len(), 1);
    }

    #[test]
    fn search_nodes_by_label() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node("t-1", NodeKind::Topic, "machine learning", None)
            .unwrap();
        kg.add_node("t-2", NodeKind::Topic, "deep learning", None)
            .unwrap();
        kg.add_node("t-3", NodeKind::Topic, "web development", None)
            .unwrap();

        let results = kg.search_nodes("learning", None).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_nodes_filtered_by_kind() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node("t-1", NodeKind::Topic, "machine learning", None)
            .unwrap();
        kg.add_node("p-1", NodeKind::Paper, "ML paper about learning", None)
            .unwrap();

        let results = kg.search_nodes("learning", Some(NodeKind::Topic)).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, NodeKind::Topic);
    }

    #[test]
    fn upsert_preserves_created_at() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node("p-1", NodeKind::Paper, "Original Label", None)
            .unwrap();
        let original = kg.get_node("p-1").unwrap().unwrap();

        // Update the label; created_at should be preserved
        kg.add_node(
            "p-1",
            NodeKind::Paper,
            "Updated Label",
            Some(r#"{"key":"val"}"#),
        )
        .unwrap();
        let updated = kg.get_node("p-1").unwrap().unwrap();

        assert_eq!(updated.label, "Updated Label");
        assert_eq!(updated.metadata_json.as_deref(), Some(r#"{"key":"val"}"#));
        assert_eq!(updated.created_at, original.created_at);
    }

    #[test]
    fn created_at_is_rfc3339() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node("ts-1", NodeKind::Topic, "Timestamp Test", None)
            .unwrap();
        let node = kg.get_node("ts-1").unwrap().unwrap();

        // Verify created_at roundtrips through serde as RFC 3339
        let json = serde_json::to_string(&node).unwrap();
        assert!(
            json.contains("T") && json.contains("Z"),
            "created_at should be RFC 3339 format, got: {json}"
        );
        let roundtripped: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.created_at, node.created_at);
    }

    /// GIVEN NodeKind::all()
    /// WHEN each variant is round-tripped through as_str + parse
    /// THEN all variants are covered and parseable
    #[test]
    fn node_kind_all_covers_every_variant() {
        use strum::EnumCount;

        let all = NodeKind::all();
        // Verify each entry round-trips through serde
        for kind in all {
            let s = kind.as_str();
            let parsed = parse_node_kind(s).unwrap();
            assert_eq!(*kind, parsed, "NodeKind round-trip failed for '{s}'");
        }
        // Verify count matches the enum variants (compile-time guarantee)
        assert_eq!(
            all.len(),
            NodeKind::COUNT,
            "NodeKind::all() length must match EnumCount"
        );
    }

    /// GIVEN EdgeKind::all()
    /// WHEN each variant is round-tripped through as_str + parse
    /// THEN all variants are covered and parseable
    #[test]
    fn edge_kind_all_covers_every_variant() {
        use strum::EnumCount;

        let all = EdgeKind::all();
        for kind in all {
            let s = kind.as_str();
            let parsed = parse_edge_kind(s).unwrap();
            assert_eq!(*kind, parsed, "EdgeKind round-trip failed for '{s}'");
        }
        assert_eq!(
            all.len(),
            EdgeKind::COUNT,
            "EdgeKind::all() length must match EnumCount"
        );
    }

    /// GIVEN a timestamp in SQLite's datetime('now') format
    /// WHEN parse_timestamp is called
    /// THEN it falls back to the legacy format and returns a valid OffsetDateTime
    #[test]
    fn parse_timestamp_accepts_legacy_sqlite_format() {
        let legacy = "2025-12-15 10:30:00";
        let dt = parse_timestamp(legacy).expect("should parse legacy format");
        assert_eq!(dt.year(), 2025);
        assert_eq!(dt.month() as u8, 12);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 10);
        assert_eq!(dt.minute(), 30);
    }

    /// GIVEN a timestamp in RFC 3339 format
    /// WHEN parse_timestamp is called
    /// THEN it parses without falling back
    #[test]
    fn parse_timestamp_accepts_rfc3339_format() {
        let rfc = "2026-04-08T12:00:00Z";
        let dt = parse_timestamp(rfc).expect("should parse RFC 3339");
        assert_eq!(dt.year(), 2026);
    }

    #[test]
    fn stats_returns_counts() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        assert_eq!(kg.stats().unwrap(), (0, 0));

        kg.add_node("a", NodeKind::Topic, "A", None).unwrap();
        kg.add_node("b", NodeKind::Paper, "B", None).unwrap();
        kg.add_edge("a", "b", EdgeKind::Extends, 1.0, None).unwrap();

        assert_eq!(kg.stats().unwrap(), (2, 1));
    }

    #[test]
    fn get_nonexistent_node_returns_none() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        assert!(kg.get_node("does-not-exist").unwrap().is_none());
    }

    #[test]
    fn edge_upsert_updates_weight() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node("a", NodeKind::Paper, "Paper A", None).unwrap();
        kg.add_node("b", NodeKind::Paper, "Paper B", None).unwrap();

        kg.add_edge("a", "b", EdgeKind::Cites, 0.5, None).unwrap();
        // Upsert same edge with higher weight
        kg.add_edge(
            "a",
            "b",
            EdgeKind::Cites,
            0.9,
            Some(r#"{"reason":"stronger link"}"#),
        )
        .unwrap();

        let edges = kg.edges_from("a").unwrap();
        assert_eq!(edges.len(), 1, "upsert should not create duplicate edge");
        assert!((edges[0].weight - 0.9).abs() < f64::EPSILON);
        assert_eq!(
            edges[0].metadata_json.as_deref(),
            Some(r#"{"reason":"stronger link"}"#)
        );
    }

    #[test]
    fn multiple_edge_types_between_same_nodes() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node("a", NodeKind::Paper, "Paper A", None).unwrap();
        kg.add_node("b", NodeKind::Paper, "Paper B", None).unwrap();

        // Same node pair, different edge types (PK is source_id + target_id + kind)
        kg.add_edge("a", "b", EdgeKind::Cites, 1.0, None).unwrap();
        kg.add_edge("a", "b", EdgeKind::Extends, 0.8, None).unwrap();
        kg.add_edge("a", "b", EdgeKind::Contradicts, 0.3, None)
            .unwrap();

        let edges = kg.edges_from("a").unwrap();
        assert_eq!(edges.len(), 3, "different edge types should coexist");

        let incoming = kg.edges_to("b").unwrap();
        assert_eq!(incoming.len(), 3);
    }

    #[test]
    fn edges_from_nonexistent_node_returns_empty() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        assert!(kg.edges_from("ghost").unwrap().is_empty());
        assert!(kg.edges_to("ghost").unwrap().is_empty());
    }

    #[test]
    fn search_no_matches_returns_empty() {
        let kg = KnowledgeGraph::open_in_memory().unwrap();
        kg.add_node("t-1", NodeKind::Topic, "machine learning", None)
            .unwrap();
        let results = kg.search_nodes("quantum", None).unwrap();
        assert!(results.is_empty());
    }
}
