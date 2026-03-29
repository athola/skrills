//! OpenAlex API client.
//! API: <https://api.openalex.org>

use crate::models::{Paper, PaperSource};
use crate::TomeResult;

const BASE_URL: &str = "https://api.openalex.org";

pub struct OpenAlexClient {
    http: reqwest::Client,
}

impl Default for OpenAlexClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenAlexClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("skrills-tome/0.1 (https://github.com/athola/skrills)")
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "OpenAlex client builder failed, falling back without User-Agent");
                    reqwest::Client::new()
                }),
        }
    }

    /// Search for works (papers) via the OpenAlex API.
    pub async fn search(&self, query: &str, limit: usize) -> TomeResult<Vec<Paper>> {
        let limit = limit.min(200);
        let resp = self
            .http
            .get(format!("{BASE_URL}/works"))
            .query(&[("search", query), ("per_page", &limit.to_string())])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(crate::TomeError::Api {
                api: "openalex".to_string(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body: serde_json::Value = resp.json().await?;
        let papers = body["results"]
            .as_array()
            .ok_or_else(|| crate::TomeError::Api {
                api: "openalex".to_string(),
                message: "response missing 'results' array".to_string(),
            })?
            .iter()
            .filter_map(parse_work)
            .collect();

        Ok(papers)
    }
}

pub(crate) fn parse_work(v: &serde_json::Value) -> Option<Paper> {
    Some(Paper {
        id: v["id"].as_str()?.to_string(),
        title: v["title"].as_str()?.to_string(),
        authors: v["authorships"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["author"]["display_name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        abstract_text: None, // OpenAlex abstracts require separate inverted_abstract_index decoding
        year: v["publication_year"].as_i64().map(|y| y as i32),
        doi: v["doi"].as_str().map(|d| d.replace("https://doi.org/", "")),
        url: v["id"].as_str().map(String::from),
        source: PaperSource::OpenAlex,
        citation_count: v["cited_by_count"].as_u64().map(|c| c as u32),
        pdf_url: v["open_access"]["oa_url"].as_str().map(String::from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_work_full() {
        let v = serde_json::json!({
            "id": "https://openalex.org/W12345",
            "title": "Deep Learning Survey",
            "authorships": [
                {"author": {"display_name": "Yann LeCun"}},
                {"author": {"display_name": "Geoffrey Hinton"}}
            ],
            "publication_year": 2015,
            "doi": "https://doi.org/10.1234/deep",
            "cited_by_count": 5000,
            "open_access": {"oa_url": "https://arxiv.org/pdf/1234.pdf"}
        });
        let p = parse_work(&v).unwrap();
        assert_eq!(p.id, "https://openalex.org/W12345");
        assert_eq!(p.title, "Deep Learning Survey");
        assert_eq!(p.authors, vec!["Yann LeCun", "Geoffrey Hinton"]);
        assert_eq!(p.year, Some(2015));
        assert_eq!(p.doi.as_deref(), Some("10.1234/deep"));
        assert_eq!(p.citation_count, Some(5000));
        assert_eq!(p.source, PaperSource::OpenAlex);
        assert_eq!(p.pdf_url.as_deref(), Some("https://arxiv.org/pdf/1234.pdf"));
    }

    #[test]
    fn parse_work_missing_id_returns_none() {
        let v = serde_json::json!({"title": "No ID"});
        assert!(parse_work(&v).is_none());
    }

    #[test]
    fn parse_work_missing_title_returns_none() {
        let v = serde_json::json!({"id": "https://openalex.org/W1"});
        assert!(parse_work(&v).is_none());
    }

    #[test]
    fn parse_work_minimal() {
        let v = serde_json::json!({
            "id": "https://openalex.org/W1",
            "title": "Minimal"
        });
        let p = parse_work(&v).unwrap();
        assert_eq!(p.title, "Minimal");
        assert!(p.authors.is_empty());
        assert!(p.year.is_none());
        assert!(p.doi.is_none());
        assert!(p.citation_count.is_none());
        assert!(p.pdf_url.is_none());
    }

    #[test]
    fn parse_work_strips_doi_prefix() {
        let v = serde_json::json!({
            "id": "https://openalex.org/W1",
            "title": "DOI Test",
            "doi": "https://doi.org/10.5555/test"
        });
        let p = parse_work(&v).unwrap();
        assert_eq!(p.doi.as_deref(), Some("10.5555/test"));
    }

    #[test]
    fn parse_work_empty_authorships() {
        let v = serde_json::json!({
            "id": "https://openalex.org/W1",
            "title": "Test",
            "authorships": []
        });
        let p = parse_work(&v).unwrap();
        assert!(p.authors.is_empty());
    }
}
