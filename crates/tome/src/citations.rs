//! Citation tracking for forward/backward citation traversal.

use crate::models::Paper;
use crate::TomeResult;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

/// A citation link between two papers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    pub citing_paper_id: String,
    pub cited_paper_id: String,
    pub context: Option<String>,
}

/// Persistent citation tracker.
pub struct CitationTracker {
    conn: Connection,
}

impl CitationTracker {
    /// Opens or creates the citation database.
    pub fn open(db_path: &std::path::Path) -> TomeResult<Self> {
        let conn = Connection::open(db_path)?;
        let ct = Self { conn };
        ct.init_schema()?;
        Ok(ct)
    }

    pub fn open_in_memory() -> TomeResult<Self> {
        let conn = Connection::open_in_memory()?;
        let ct = Self { conn };
        ct.init_schema()?;
        Ok(ct)
    }

    fn init_schema(&self) -> TomeResult<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tracked_papers (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                doi TEXT,
                year INTEGER,
                citation_count INTEGER DEFAULT 0,
                last_checked TEXT,
                metadata_json TEXT
            );

            CREATE TABLE IF NOT EXISTS citations (
                citing_id TEXT NOT NULL,
                cited_id TEXT NOT NULL,
                context TEXT,
                discovered_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (citing_id, cited_id)
            );

            CREATE INDEX IF NOT EXISTS idx_citations_citing ON citations(citing_id);
            CREATE INDEX IF NOT EXISTS idx_citations_cited ON citations(cited_id);
            CREATE INDEX IF NOT EXISTS idx_papers_doi ON tracked_papers(doi);",
        )?;
        Ok(())
    }

    /// Track a paper for citation monitoring.
    pub fn track_paper(&self, paper: &Paper) -> TomeResult<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO tracked_papers (id, title, doi, year, citation_count)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                paper.id,
                paper.title,
                paper.doi,
                paper.year,
                paper.citation_count
            ],
        )?;
        Ok(())
    }

    /// Record a citation relationship.
    pub fn add_citation(
        &self,
        citing_id: &str,
        cited_id: &str,
        context: Option<&str>,
    ) -> TomeResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO citations (citing_id, cited_id, context) VALUES (?1, ?2, ?3)",
            rusqlite::params![citing_id, cited_id, context],
        )?;
        Ok(())
    }

    /// Forward citations: "who cited this paper?"
    pub fn forward_citations(&self, paper_id: &str) -> TomeResult<Vec<Citation>> {
        let mut stmt = self
            .conn
            .prepare("SELECT citing_id, cited_id, context FROM citations WHERE cited_id = ?1")?;
        let rows = stmt.query_map([paper_id], |row| {
            Ok(Citation {
                citing_paper_id: row.get(0)?,
                cited_paper_id: row.get(1)?,
                context: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(crate::TomeError::Cache)
    }

    /// Backward citations: "what does this paper cite?"
    pub fn backward_citations(&self, paper_id: &str) -> TomeResult<Vec<Citation>> {
        let mut stmt = self
            .conn
            .prepare("SELECT citing_id, cited_id, context FROM citations WHERE citing_id = ?1")?;
        let rows = stmt.query_map([paper_id], |row| {
            Ok(Citation {
                citing_paper_id: row.get(0)?,
                cited_paper_id: row.get(1)?,
                context: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(crate::TomeError::Cache)
    }

    /// Get all tracked papers.
    pub fn tracked_papers(&self) -> TomeResult<Vec<(String, String, Option<u32>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, citation_count FROM tracked_papers ORDER BY citation_count DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(crate::TomeError::Cache)
    }

    /// Count tracked papers and citation links.
    pub fn stats(&self) -> TomeResult<(usize, usize)> {
        let papers: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracked_papers", [], |r| r.get(0))?;
        let citations: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM citations", [], |r| r.get(0))?;
        Ok((papers as usize, citations as usize))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Paper, PaperSource};

    fn test_paper(id: &str, title: &str) -> Paper {
        Paper {
            id: id.to_string(),
            title: title.to_string(),
            authors: vec![],
            abstract_text: None,
            year: Some(2024),
            doi: Some(format!("10.1234/{id}")),
            url: None,
            source: PaperSource::SemanticScholar,
            citation_count: Some(42),
            pdf_url: None,
        }
    }

    #[test]
    fn track_and_list_papers() {
        let ct = CitationTracker::open_in_memory().unwrap();
        ct.track_paper(&test_paper("p1", "Transformers")).unwrap();
        ct.track_paper(&test_paper("p2", "BERT")).unwrap();
        let papers = ct.tracked_papers().unwrap();
        assert_eq!(papers.len(), 2);
    }

    #[test]
    fn forward_and_backward_citations() {
        let ct = CitationTracker::open_in_memory().unwrap();
        ct.track_paper(&test_paper("p1", "Original")).unwrap();
        ct.track_paper(&test_paper("p2", "Citing Paper")).unwrap();
        ct.add_citation("p2", "p1", Some("extends the transformer"))
            .unwrap();

        let fwd = ct.forward_citations("p1").unwrap();
        assert_eq!(fwd.len(), 1);
        assert_eq!(fwd[0].citing_paper_id, "p2");

        let bwd = ct.backward_citations("p2").unwrap();
        assert_eq!(bwd.len(), 1);
        assert_eq!(bwd[0].cited_paper_id, "p1");
    }

    #[test]
    fn stats_counts() {
        let ct = CitationTracker::open_in_memory().unwrap();
        assert_eq!(ct.stats().unwrap(), (0, 0));
        ct.track_paper(&test_paper("p1", "A")).unwrap();
        ct.track_paper(&test_paper("p2", "B")).unwrap();
        ct.add_citation("p2", "p1", None).unwrap();
        assert_eq!(ct.stats().unwrap(), (2, 1));
    }
}
