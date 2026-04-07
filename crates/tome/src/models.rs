//! Data models for research results.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// A research paper from any source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Paper {
    /// Unique identifier (DOI preferred, else source-specific ID)
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    #[serde(rename = "abstract")]
    pub abstract_text: Option<String>,
    pub year: Option<i32>,
    pub doi: Option<String>,
    pub url: Option<String>,
    pub source: PaperSource,
    pub citation_count: Option<u32>,
    pub pdf_url: Option<String>,
}

/// Source API that returned the paper.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PaperSource {
    SemanticScholar,
    Arxiv,
    OpenAlex,
    CrossRef,
}

/// A community discussion result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Discussion {
    pub id: String,
    pub title: String,
    pub url: String,
    pub points: Option<u32>,
    pub comment_count: Option<u32>,
    pub source: DiscussionSource,
    #[serde(with = "time::serde::rfc3339::option")]
    pub created_at: Option<OffsetDateTime>,
}

/// Source of the discussion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiscussionSource {
    HackerNews,
}

/// DOI resolution result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoiMetadata {
    pub doi: String,
    pub title: String,
    pub authors: Vec<String>,
    pub publisher: Option<String>,
    pub year: Option<i32>,
    pub url: Option<String>,
    pub journal: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discussion_roundtrip_with_created_at() {
        let ts = time::OffsetDateTime::parse(
            "2024-06-15T10:30:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();

        let d = Discussion {
            id: "1".to_string(),
            title: "Test".to_string(),
            url: "https://example.com".to_string(),
            points: Some(42),
            comment_count: Some(10),
            source: DiscussionSource::HackerNews,
            created_at: Some(ts),
        };

        let json = serde_json::to_string(&d).unwrap();
        assert!(
            json.contains("2024-06-15T10:30:00Z"),
            "JSON should contain RFC 3339 timestamp"
        );

        let roundtripped: Discussion = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.created_at, Some(ts));
    }

    #[test]
    fn discussion_roundtrip_with_none_created_at() {
        let d = Discussion {
            id: "2".to_string(),
            title: "No Date".to_string(),
            url: "https://example.com".to_string(),
            points: None,
            comment_count: None,
            source: DiscussionSource::HackerNews,
            created_at: None,
        };

        let json = serde_json::to_string(&d).unwrap();
        let roundtripped: Discussion = serde_json::from_str(&json).unwrap();
        assert!(roundtripped.created_at.is_none());
    }

    #[test]
    fn paper_source_serde_snake_case() {
        let json = serde_json::to_string(&PaperSource::SemanticScholar).unwrap();
        assert_eq!(json, "\"semantic_scholar\"");
        let roundtripped: PaperSource = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped, PaperSource::SemanticScholar);
    }

    #[test]
    fn discussion_source_serde_snake_case() {
        let json = serde_json::to_string(&DiscussionSource::HackerNews).unwrap();
        assert_eq!(json, "\"hacker_news\"");
        let roundtripped: DiscussionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped, DiscussionSource::HackerNews);
    }

    #[test]
    fn research_session_roundtrip() {
        let ts = time::OffsetDateTime::parse(
            "2024-06-15T10:30:00Z",
            &time::format_description::well_known::Rfc3339,
        )
        .unwrap();

        let session = ResearchSession {
            id: "sess-1".to_string(),
            query: "rust async".to_string(),
            created_at: ts,
            paper_count: 5,
            discussion_count: 3,
        };

        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("2024-06-15T10:30:00Z"));

        let roundtripped: ResearchSession = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.created_at, ts);
        assert_eq!(roundtripped.paper_count, 5);
    }

    #[test]
    fn research_session_legacy_timestamp_field_rejected() {
        // Old format used field name "timestamp" with a plain string.
        // After migration to created_at: OffsetDateTime, old JSON is
        // a breaking change — verify it fails cleanly rather than panicking.
        let old_json = r#"{
            "id": "sess-old",
            "query": "test",
            "timestamp": "2024-01-01 00:00:00",
            "paper_count": 1,
            "discussion_count": 0
        }"#;
        let result = serde_json::from_str::<ResearchSession>(old_json);
        assert!(
            result.is_err(),
            "Old 'timestamp' field should not deserialize into new struct"
        );
    }
}

/// A cached research session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSession {
    pub id: String,
    pub query: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub paper_count: usize,
    pub discussion_count: usize,
}
