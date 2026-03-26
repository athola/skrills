//! arXiv API client.
//! API: <https://export.arxiv.org/api/query>

use crate::models::{Paper, PaperSource};
use crate::TomeResult;

const BASE_URL: &str = "https://export.arxiv.org/api/query";

pub struct ArxivClient {
    http: reqwest::Client,
}

impl ArxivClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("skrills-tome/0.1 (https://github.com/athola/skrills)")
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "ArXiv client builder failed, using default");
                    reqwest::Client::new()
                }),
        }
    }

    /// Search arXiv for papers. Returns parsed Atom XML results.
    pub async fn search(&self, query: &str, limit: usize) -> TomeResult<Vec<Paper>> {
        let limit = limit.min(100);
        // Sanitize query: strip arXiv field-prefix syntax to prevent injection
        let sanitized = query
            .split_whitespace()
            .map(|word| {
                // Strip field prefixes like "ti:", "au:", "abs:", "all:", etc.
                if let Some((_prefix, rest)) = word.split_once(':') {
                    rest
                } else {
                    word
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let resp = self
            .http
            .get(BASE_URL)
            .query(&[
                ("search_query", &format!("all:{sanitized}")),
                ("max_results", &limit.to_string()),
                ("sortBy", &"relevance".to_string()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(crate::TomeError::Api {
                api: "arxiv".to_string(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body = resp.text().await?;
        Ok(parse_arxiv_atom(&body))
    }
}

/// Minimal Atom XML parser for arXiv results (no XML crate dependency).
fn parse_arxiv_atom(xml: &str) -> Vec<Paper> {
    let mut papers = Vec::new();

    for entry in xml.split("<entry>").skip(1) {
        let title = extract_tag(entry, "title").map(|t| t.replace('\n', " ").trim().to_string());
        let id = extract_tag(entry, "id");
        let summary = extract_tag(entry, "summary").map(|s| s.trim().to_string());
        let published = extract_tag(entry, "published");

        if let (Some(title), Some(id)) = (title, id) {
            let year = published.and_then(|p| p.get(..4).and_then(|y| y.parse().ok()));

            // Extract arXiv ID from URL
            let arxiv_id = id.rsplit('/').next().unwrap_or(&id).to_string();

            papers.push(Paper {
                id: arxiv_id.clone(),
                title,
                authors: extract_authors(entry),
                abstract_text: summary,
                year,
                doi: None,
                url: Some(id.to_string()),
                source: PaperSource::Arxiv,
                citation_count: None,
                pdf_url: Some(format!("https://arxiv.org/pdf/{arxiv_id}")),
            });
        }
    }

    papers
}

fn extract_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let start = xml.find(&open)?;
    let after_open = xml[start..].find('>')? + start + 1;
    let end = xml[after_open..].find(&close)? + after_open;
    Some(xml[after_open..end].to_string())
}

fn extract_authors(entry: &str) -> Vec<String> {
    entry
        .split("<author>")
        .skip(1)
        .filter_map(|a| extract_tag(a, "name"))
        .collect()
}
