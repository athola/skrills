//! Data models for research results.

use serde::{Deserialize, Serialize};

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
    pub created_at: Option<String>,
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

/// A cached research session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSession {
    pub id: String,
    pub query: String,
    pub timestamp: String,
    pub paper_count: usize,
    pub discussion_count: usize,
}
