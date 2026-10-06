//! Hacker News Algolia API client.
//! API: <https://hn.algolia.com/api/v1>

use crate::models::{Discussion, DiscussionSource};
use crate::TomeResult;

const BASE_URL: &str = "https://hn.algolia.com/api/v1";

pub struct HnAlgoliaClient {
    http: reqwest::Client,
}

impl Default for HnAlgoliaClient {
    fn default() -> Self {
        Self::new()
    }
}

impl HnAlgoliaClient {
    pub fn new() -> Self {
        Self {
            http: super::http_client("hn_algolia", None),
        }
    }

    /// Search HN stories and comments.
    pub async fn search(&self, query: &str, limit: usize) -> TomeResult<Vec<Discussion>> {
        let resp = self.http.execute(self.request(query, limit)?).await?;

        let resp = super::ensure_success("hn_algolia", resp, "")?;

        let body = super::read_json("hn_algolia", resp).await?;
        let discussions = body["hits"]
            .as_array()
            .ok_or_else(|| crate::TomeError::Api {
                api: "hn_algolia".to_string(),
                message: "response missing 'hits' array".to_string(),
            })?
            .iter()
            .filter_map(parse_hit)
            .collect();

        Ok(discussions)
    }
}

impl HnAlgoliaClient {
    fn request(&self, query: &str, limit: usize) -> reqwest::Result<reqwest::Request> {
        self.http
            .get(format!("{BASE_URL}/search"))
            .query(&[
                ("query", query),
                ("tags", "story"),
                ("hitsPerPage", &limit.min(100).to_string()),
            ])
            .build()
    }
}

pub(crate) fn parse_hit(v: &serde_json::Value) -> Option<Discussion> {
    let id = v["objectID"].as_str()?;
    Some(Discussion {
        id: id.to_string(),
        title: v["title"].as_str()?.to_string(),
        url: v["url"]
            .as_str()
            .map(String::from)
            .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={id}")),
        points: v["points"].as_u64().map(|p| p as u32),
        comment_count: v["num_comments"].as_u64().map(|c| c as u32),
        source: DiscussionSource::HackerNews,
        created_at: v["created_at"].as_str().and_then(|s| {
            time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339)
                .inspect_err(|e| {
                    tracing::debug!("failed to parse created_at timestamp {:?}: {}", s, e);
                })
                .ok()
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hits_per_page_is_capped_like_the_other_clients() {
        // IN-51: the limit used to be forwarded uncapped.
        let req = HnAlgoliaClient::new().request("rust", 100_000).unwrap();
        let hits = req
            .url()
            .query_pairs()
            .find(|(k, _)| k == "hitsPerPage")
            .map(|(_, v)| v.into_owned());
        assert_eq!(hits.as_deref(), Some("100"));
    }

    #[test]
    fn parse_hit_full() {
        let v = serde_json::json!({
            "objectID": "12345",
            "title": "Show HN: My Rust Project",
            "url": "https://example.com/project",
            "points": 142,
            "num_comments": 53,
            "created_at": "2024-06-15T10:30:00Z"
        });
        let d = parse_hit(&v).unwrap();
        assert_eq!(d.id, "12345");
        assert_eq!(d.title, "Show HN: My Rust Project");
        assert_eq!(d.url, "https://example.com/project");
        assert_eq!(d.points, Some(142));
        assert_eq!(d.comment_count, Some(53));
        assert_eq!(d.source, DiscussionSource::HackerNews);
        assert!(d.created_at.is_some());
    }

    #[test]
    fn parse_hit_missing_url_uses_hn_link() {
        let v = serde_json::json!({
            "objectID": "99",
            "title": "Ask HN: Something"
        });
        let d = parse_hit(&v).unwrap();
        assert_eq!(d.url, "https://news.ycombinator.com/item?id=99");
    }

    #[test]
    fn parse_hit_missing_title_returns_none() {
        let v = serde_json::json!({"objectID": "1"});
        assert!(parse_hit(&v).is_none());
    }

    #[test]
    fn parse_hit_missing_id_returns_none() {
        let v = serde_json::json!({"title": "No ID"});
        assert!(parse_hit(&v).is_none());
    }

    #[test]
    fn parse_hit_invalid_created_at() {
        let v = serde_json::json!({
            "objectID": "1",
            "title": "Test",
            "created_at": "not-a-date"
        });
        let d = parse_hit(&v).unwrap();
        assert!(d.created_at.is_none());
    }

    #[test]
    fn parse_hit_optional_numeric_fields() {
        let v = serde_json::json!({
            "objectID": "1",
            "title": "Test"
        });
        let d = parse_hit(&v).unwrap();
        assert!(d.points.is_none());
        assert!(d.comment_count.is_none());
        assert!(d.created_at.is_none());
    }
}
