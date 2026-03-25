//! Semantic Scholar API client.
//! API: <https://api.semanticscholar.org>

use crate::models::Paper;
use crate::TomeResult;

const BASE_URL: &str = "https://api.semanticscholar.org/graph/v1";

pub struct SemanticScholarClient {
    http: reqwest::Client,
}

impl SemanticScholarClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Search for papers by query string.
    pub async fn search(&self, query: &str, limit: usize) -> TomeResult<Vec<Paper>> {
        let url = format!("{BASE_URL}/paper/search");
        let resp = self
            .http
            .get(&url)
            .query(&[("query", query), ("limit", &limit.to_string())])
            .query(&[(
                "fields",
                "title,authors,abstract,year,externalIds,citationCount,openAccessPdf",
            )])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(crate::TomeError::Api {
                api: "semantic_scholar".to_string(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body: serde_json::Value = resp.json().await?;
        let papers = body["data"]
            .as_array()
            .ok_or_else(|| crate::TomeError::Api {
                api: "semantic_scholar".to_string(),
                message: "response missing 'data' array".to_string(),
            })?
            .iter()
            .filter_map(|p| parse_s2_paper(p))
            .collect();

        Ok(papers)
    }
}

fn parse_s2_paper(v: &serde_json::Value) -> Option<Paper> {
    Some(Paper {
        id: v["paperId"].as_str()?.to_string(),
        title: v["title"].as_str()?.to_string(),
        authors: v["authors"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        abstract_text: v["abstract"].as_str().map(String::from),
        year: v["year"].as_i64().map(|y| y as i32),
        doi: v["externalIds"]["DOI"].as_str().map(String::from),
        url: Some(format!(
            "https://www.semanticscholar.org/paper/{}",
            v["paperId"].as_str()?
        )),
        source: crate::models::PaperSource::SemanticScholar,
        citation_count: v["citationCount"].as_u64().map(|c| c as u32),
        pdf_url: v["openAccessPdf"]["url"].as_str().map(String::from),
    })
}
