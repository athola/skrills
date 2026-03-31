//! arXiv API client.
//! API: <https://export.arxiv.org/api/query>

use crate::models::{Paper, PaperSource};
use crate::TomeResult;

const BASE_URL: &str = "https://export.arxiv.org/api/query";

pub struct ArxivClient {
    http: reqwest::Client,
}

impl Default for ArxivClient {
    fn default() -> Self {
        Self::new()
    }
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
        let sanitized = sanitize_query(query);

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

/// Strip arXiv field-prefix syntax (e.g. `ti:`, `au:`) from a query string
/// to prevent users from altering search semantics.
fn sanitize_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|word| {
            if let Some((_prefix, rest)) = word.split_once(':') {
                rest
            } else {
                word
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_field_prefixes() {
        assert_eq!(sanitize_query("ti:quantum au:einstein"), "quantum einstein");
        assert_eq!(sanitize_query("abs:neural all:network"), "neural network");
    }

    #[test]
    fn sanitize_preserves_plain_queries() {
        assert_eq!(sanitize_query("machine learning"), "machine learning");
        assert_eq!(sanitize_query("rust async runtime"), "rust async runtime");
    }

    #[test]
    fn sanitize_handles_edge_cases() {
        assert_eq!(sanitize_query(""), "");
        assert_eq!(sanitize_query("ti:"), "");
        assert_eq!(sanitize_query("word:with:colons"), "with:colons");
    }

    #[test]
    fn parse_arxiv_atom_empty() {
        assert!(parse_arxiv_atom("").is_empty());
        assert!(parse_arxiv_atom("<feed></feed>").is_empty());
    }

    #[test]
    fn parse_arxiv_atom_single_entry() {
        let xml = r#"<feed>
            <entry>
                <id>http://arxiv.org/abs/2301.00001v1</id>
                <title>Test Paper Title</title>
                <summary>A test abstract.</summary>
                <published>2023-01-01T00:00:00Z</published>
                <author><name>Test Author</name></author>
            </entry>
        </feed>"#;
        let papers = parse_arxiv_atom(xml);
        assert_eq!(papers.len(), 1);
        assert_eq!(papers[0].title, "Test Paper Title");
        assert_eq!(papers[0].id, "2301.00001v1");
        assert_eq!(papers[0].authors, vec!["Test Author"]);
        assert_eq!(papers[0].year, Some(2023));
    }

    #[test]
    fn parse_arxiv_atom_multiple_entries() {
        let xml = r#"<feed>
            <entry>
                <id>http://arxiv.org/abs/2301.00001v1</id>
                <title>First Paper</title>
                <summary>Abstract one.</summary>
                <published>2023-01-01T00:00:00Z</published>
                <author><name>Author A</name></author>
            </entry>
            <entry>
                <id>http://arxiv.org/abs/2301.00002v1</id>
                <title>Second Paper</title>
                <summary>Abstract two.</summary>
                <published>2024-06-15T00:00:00Z</published>
                <author><name>Author B</name></author>
                <author><name>Author C</name></author>
            </entry>
        </feed>"#;
        let papers = parse_arxiv_atom(xml);
        assert_eq!(papers.len(), 2);
        assert_eq!(papers[1].title, "Second Paper");
        assert_eq!(papers[1].authors, vec!["Author B", "Author C"]);
        assert_eq!(papers[1].year, Some(2024));
    }

    #[test]
    fn parse_arxiv_atom_missing_summary() {
        let xml = r#"<feed>
            <entry>
                <id>http://arxiv.org/abs/2301.00001v1</id>
                <title>No Abstract</title>
                <published>2023-01-01T00:00:00Z</published>
                <author><name>Author</name></author>
            </entry>
        </feed>"#;
        let papers = parse_arxiv_atom(xml);
        assert_eq!(papers.len(), 1);
        assert!(papers[0].abstract_text.is_none());
    }

    #[test]
    fn parse_arxiv_atom_entry_missing_id_skipped() {
        let xml = r#"<feed>
            <entry>
                <title>No ID Paper</title>
                <summary>Some text.</summary>
            </entry>
        </feed>"#;
        let papers = parse_arxiv_atom(xml);
        assert!(papers.is_empty());
    }

    #[test]
    fn parse_arxiv_pdf_url_generated() {
        let xml = r#"<feed>
            <entry>
                <id>http://arxiv.org/abs/2301.99999v2</id>
                <title>PDF Test</title>
                <published>2023-01-01T00:00:00Z</published>
                <author><name>A</name></author>
            </entry>
        </feed>"#;
        let papers = parse_arxiv_atom(xml);
        assert_eq!(
            papers[0].pdf_url.as_deref(),
            Some("https://arxiv.org/pdf/2301.99999v2")
        );
    }
}
