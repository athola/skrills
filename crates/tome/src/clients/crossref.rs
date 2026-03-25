//! CrossRef API client for DOI resolution.
//! API: <https://api.crossref.org>

use crate::models::DoiMetadata;
use crate::TomeResult;

const BASE_URL: &str = "https://api.crossref.org";

fn encode_doi(doi: &str) -> String {
    url::form_urlencoded::byte_serialize(doi.as_bytes()).collect()
}

pub struct CrossRefClient {
    http: reqwest::Client,
}

impl CrossRefClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("skrills-tome/0.1 (https://github.com/athola/skrills; mailto:research@skrills.dev)")
                .build()
                .unwrap_or_else(|e| {
                    tracing::warn!(error = %e, "CrossRef client builder failed, falling back without User-Agent");
                    reqwest::Client::new()
                }),
        }
    }

    /// Resolve a DOI to full metadata.
    pub async fn resolve_doi(&self, doi: &str) -> TomeResult<DoiMetadata> {
        let resp = self
            .http
            .get(&format!("{BASE_URL}/works/{}", encode_doi(doi)))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(crate::TomeError::Api {
                api: "crossref".to_string(),
                message: format!("HTTP {} for DOI {doi}", resp.status()),
            });
        }

        let body: serde_json::Value = resp.json().await?;
        let msg = &body["message"];

        Ok(DoiMetadata {
            doi: doi.to_string(),
            title: msg["title"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|t| t.as_str())
                .unwrap_or("Unknown")
                .to_string(),
            authors: msg["author"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| {
                            let given = x["given"].as_str().unwrap_or("");
                            let family = x["family"].as_str().unwrap_or("");
                            if family.is_empty() {
                                None
                            } else {
                                Some(format!("{given} {family}").trim().to_string())
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
            publisher: msg["publisher"].as_str().map(String::from),
            year: msg["published-print"]["date-parts"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|y| y.as_i64())
                .map(|y| y as i32),
            url: msg["URL"].as_str().map(String::from),
            journal: msg["container-title"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|t| t.as_str())
                .map(String::from),
        })
    }
}
