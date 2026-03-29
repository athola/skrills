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

impl Default for CrossRefClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CrossRefClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent("skrills-tome/0.1 (https://github.com/athola/skrills; mailto:research@skrills.dev)")
                .timeout(std::time::Duration::from_secs(30))
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
            .get(format!("{BASE_URL}/works/{}", encode_doi(doi)))
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(crate::TomeError::Api {
                api: "crossref".to_string(),
                message: format!("HTTP {} for DOI {doi}", resp.status()),
            });
        }

        let body: serde_json::Value = resp.json().await?;
        Ok(parse_crossref_message(doi, &body["message"]))
    }
}

pub(crate) fn parse_crossref_message(doi: &str, msg: &serde_json::Value) -> DoiMetadata {
    DoiMetadata {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_crossref_message() {
        let msg: serde_json::Value = serde_json::json!({
            "title": ["Attention Is All You Need"],
            "author": [
                {"given": "Ashish", "family": "Vaswani"},
                {"given": "Noam", "family": "Shazeer"}
            ],
            "publisher": "Springer",
            "published-print": {"date-parts": [[2017]]},
            "URL": "https://doi.org/10.1234/test",
            "container-title": ["NeurIPS"]
        });
        let meta = parse_crossref_message("10.1234/test", &msg);
        assert_eq!(meta.doi, "10.1234/test");
        assert_eq!(meta.title, "Attention Is All You Need");
        assert_eq!(meta.authors, vec!["Ashish Vaswani", "Noam Shazeer"]);
        assert_eq!(meta.publisher.as_deref(), Some("Springer"));
        assert_eq!(meta.year, Some(2017));
        assert_eq!(meta.url.as_deref(), Some("https://doi.org/10.1234/test"));
        assert_eq!(meta.journal.as_deref(), Some("NeurIPS"));
    }

    #[test]
    fn parse_crossref_missing_optional_fields() {
        let msg: serde_json::Value = serde_json::json!({});
        let meta = parse_crossref_message("10.0000/empty", &msg);
        assert_eq!(meta.title, "Unknown");
        assert!(meta.authors.is_empty());
        assert!(meta.publisher.is_none());
        assert!(meta.year.is_none());
        assert!(meta.url.is_none());
        assert!(meta.journal.is_none());
    }

    #[test]
    fn parse_crossref_author_missing_family() {
        let msg: serde_json::Value = serde_json::json!({
            "title": ["Test"],
            "author": [
                {"given": "Alice", "family": "Smith"},
                {"given": "Bob", "family": ""}
            ]
        });
        let meta = parse_crossref_message("10.0/x", &msg);
        assert_eq!(meta.authors, vec!["Alice Smith"]);
    }

    #[test]
    fn encode_doi_special_chars() {
        assert_eq!(encode_doi("10.1234/test"), "10.1234%2Ftest");
    }
}
