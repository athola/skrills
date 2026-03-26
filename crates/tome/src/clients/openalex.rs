//! OpenAlex API client.
//! API: <https://api.openalex.org>

use crate::models::{Paper, PaperSource};
use crate::TomeResult;

const BASE_URL: &str = "https://api.openalex.org";

pub struct OpenAlexClient {
    http: reqwest::Client,
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
            .get(&format!("{BASE_URL}/works"))
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
            .filter_map(|w| parse_work(w))
            .collect();

        Ok(papers)
    }
}

fn parse_work(v: &serde_json::Value) -> Option<Paper> {
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
