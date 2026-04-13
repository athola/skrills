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
    conn: std::sync::Mutex<Connection>,
}

impl CitationTracker {
    /// Opens or creates the citation database.
    pub fn open(db_path: &std::path::Path) -> TomeResult<Self> {
        let conn = Connection::open(db_path)?;
        let ct = Self {
            conn: std::sync::Mutex::new(conn),
        };
        ct.init_schema()?;
        Ok(ct)
    }

    pub fn open_in_memory() -> TomeResult<Self> {
        let conn = Connection::open_in_memory()?;
        let ct = Self {
            conn: std::sync::Mutex::new(conn),
        };
        ct.init_schema()?;
        Ok(ct)
    }

    // NOTE: CREATE TABLE IF NOT EXISTS means FK constraints only apply to new
    // databases. Pre-existing databases keep the old schema without FKs.
    // Acceptable pre-1.0; a migration step should be added before 1.0 release.
    fn init_schema(&self) -> TomeResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS tracked_papers (
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
                PRIMARY KEY (citing_id, cited_id),
                FOREIGN KEY (citing_id) REFERENCES tracked_papers(id),
                FOREIGN KEY (cited_id) REFERENCES tracked_papers(id)
            );

            CREATE INDEX IF NOT EXISTS idx_citations_citing ON citations(citing_id);
            CREATE INDEX IF NOT EXISTS idx_citations_cited ON citations(cited_id);
            CREATE INDEX IF NOT EXISTS idx_papers_doi ON tracked_papers(doi);",
        )?;
        Ok(())
    }

    /// Track a paper for citation monitoring.
    pub fn track_paper(&self, paper: &Paper) -> TomeResult<()> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.execute(
            "INSERT INTO tracked_papers (id, title, doi, year, citation_count)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET title=excluded.title, doi=excluded.doi, year=excluded.year, citation_count=excluded.citation_count",
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
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let rows = conn.execute(
            "INSERT OR IGNORE INTO citations (citing_id, cited_id, context) VALUES (?1, ?2, ?3)",
            rusqlite::params![citing_id, cited_id, context],
        )?;
        if rows == 0 {
            tracing::debug!(
                "citation {citing_id} -> {cited_id} not inserted (duplicate or FK violation)"
            );
        }
        Ok(())
    }

    /// Forward citations: "who cited this paper?"
    pub fn forward_citations(&self, paper_id: &str) -> TomeResult<Vec<Citation>> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt =
            conn.prepare("SELECT citing_id, cited_id, context FROM citations WHERE cited_id = ?1")?;
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
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn
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
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn.prepare(
            "SELECT id, title, citation_count FROM tracked_papers ORDER BY citation_count DESC",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(crate::TomeError::Cache)
    }

    /// Count tracked papers and citation links.
    pub fn stats(&self) -> TomeResult<(usize, usize)> {
        let conn = self.conn.lock().unwrap_or_else(|p| p.into_inner());
        let papers: i64 =
            conn.query_row("SELECT COUNT(*) FROM tracked_papers", [], |r| r.get(0))?;
        let citations: i64 = conn.query_row("SELECT COUNT(*) FROM citations", [], |r| r.get(0))?;
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

    #[test]
    fn duplicate_citation_is_ignored() {
        let ct = CitationTracker::open_in_memory().unwrap();
        ct.track_paper(&test_paper("p1", "Original")).unwrap();
        ct.track_paper(&test_paper("p2", "Citing")).unwrap();

        ct.add_citation("p2", "p1", Some("first insert")).unwrap();
        // Second insert with same IDs should be silently ignored
        ct.add_citation("p2", "p1", Some("duplicate")).unwrap();

        let fwd = ct.forward_citations("p1").unwrap();
        assert_eq!(fwd.len(), 1, "duplicate citation should be ignored");
        assert_eq!(
            fwd[0].context.as_deref(),
            Some("first insert"),
            "original context should be preserved"
        );
    }

    #[test]
    fn upsert_updates_existing_paper() {
        let ct = CitationTracker::open_in_memory().unwrap();
        ct.track_paper(&test_paper("p1", "Original Title")).unwrap();

        // Track same paper ID with updated title
        let updated = Paper {
            id: "p1".to_string(),
            title: "Updated Title".to_string(),
            authors: vec![],
            abstract_text: None,
            year: Some(2025),
            doi: Some("10.1234/p1".to_string()),
            url: None,
            source: PaperSource::SemanticScholar,
            citation_count: Some(100),
            pdf_url: None,
        };
        ct.track_paper(&updated).unwrap();

        let papers = ct.tracked_papers().unwrap();
        assert_eq!(papers.len(), 1, "upsert should not create duplicate");
        assert_eq!(papers[0].1, "Updated Title");
        assert_eq!(papers[0].2, Some(100));
    }

    #[test]
    fn empty_tracker_returns_empty_results() {
        let ct = CitationTracker::open_in_memory().unwrap();
        assert!(ct.forward_citations("nonexistent").unwrap().is_empty());
        assert!(ct.backward_citations("nonexistent").unwrap().is_empty());
        assert!(ct.tracked_papers().unwrap().is_empty());
    }

    #[test]
    fn multiple_forward_citations() {
        let ct = CitationTracker::open_in_memory().unwrap();
        ct.track_paper(&test_paper("original", "Seminal Paper"))
            .unwrap();
        ct.track_paper(&test_paper("c1", "Citing A")).unwrap();
        ct.track_paper(&test_paper("c2", "Citing B")).unwrap();
        ct.track_paper(&test_paper("c3", "Citing C")).unwrap();

        ct.add_citation("c1", "original", None).unwrap();
        ct.add_citation("c2", "original", None).unwrap();
        ct.add_citation("c3", "original", None).unwrap();

        let fwd = ct.forward_citations("original").unwrap();
        assert_eq!(fwd.len(), 3, "should have 3 forward citations");

        // Original paper should have no backward citations
        let bwd = ct.backward_citations("original").unwrap();
        assert!(bwd.is_empty());
    }

    #[test]
    fn tracked_papers_ordered_by_citation_count() {
        let ct = CitationTracker::open_in_memory().unwrap();

        let mut low = test_paper("low", "Low Citations");
        low.citation_count = Some(5);
        let mut high = test_paper("high", "High Citations");
        high.citation_count = Some(500);
        let mut mid = test_paper("mid", "Mid Citations");
        mid.citation_count = Some(50);

        ct.track_paper(&low).unwrap();
        ct.track_paper(&high).unwrap();
        ct.track_paper(&mid).unwrap();

        let papers = ct.tracked_papers().unwrap();
        assert_eq!(papers[0].0, "high");
        assert_eq!(papers[1].0, "mid");
        assert_eq!(papers[2].0, "low");
    }
}
