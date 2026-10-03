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
            http: super::http_client("crossref", Some("skrills-tome/0.1 (https://github.com/athola/skrills; mailto:research@skrills.dev)")),
        }
    }

    /// Resolve a DOI to full metadata.
    pub async fn resolve_doi(&self, doi: &str) -> TomeResult<DoiMetadata> {
        let resp = self
            .http
            .get(format!("{BASE_URL}/works/{}", encode_doi(doi)))
            .send()
            .await?;

        let resp = super::ensure_success("crossref", resp, &format!(" for DOI {doi}"))?;
        let body = super::read_json("crossref", resp).await?;
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
        year: ["published-print", "published", "issued", "published-online"]
            .iter()
            .find_map(|field| date_year(&msg[*field])),
        url: msg["URL"].as_str().map(String::from),
        journal: msg["container-title"]
            .as_array()
            .and_then(|a| a.first())
            .and_then(|t| t.as_str())
            .map(String::from),
    }
}

/// Year from a CrossRef date object (`{"date-parts": [[2021, 5]]}`).
fn date_year(date: &serde_json::Value) -> Option<i32> {
    date["date-parts"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|y| y.as_i64())
        .and_then(|y| i32::try_from(y).ok())
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
    fn year_falls_back_when_published_print_is_absent() {
        // IN-54: online-only works carry no published-print.
        for field in ["published", "issued", "published-online"] {
            let msg = serde_json::json!({ field: {"date-parts": [[2021, 5]]} });
            let meta = parse_crossref_message("10.0/y", &msg);
            assert_eq!(meta.year, Some(2021), "field {field}");
        }
        let msg = serde_json::json!({
            "published-print": {"date-parts": [[2019]]},
            "published-online": {"date-parts": [[2018]]}
        });
        assert_eq!(parse_crossref_message("10.0/y", &msg).year, Some(2019));
        // CrossRef sends [[null]] when the date is unknown.
        let msg = serde_json::json!({
            "published-print": {"date-parts": [[null]]},
            "issued": {"date-parts": [[2020]]}
        });
        assert_eq!(parse_crossref_message("10.0/y", &msg).year, Some(2020));
    }

    #[test]
    fn encode_doi_special_chars() {
        assert_eq!(encode_doi("10.1234/test"), "10.1234%2Ftest");
    }
}
