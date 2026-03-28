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
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Search HN stories and comments.
    pub async fn search(&self, query: &str, limit: usize) -> TomeResult<Vec<Discussion>> {
        let resp = self
            .http
            .get(format!("{BASE_URL}/search"))
            .query(&[
                ("query", query),
                ("tags", "story"),
                ("hitsPerPage", &limit.to_string()),
            ])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err(crate::TomeError::Api {
                api: "hn_algolia".to_string(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        let body: serde_json::Value = resp.json().await?;
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

fn parse_hit(v: &serde_json::Value) -> Option<Discussion> {
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
        created_at: v["created_at"].as_str().map(String::from),
    })
}
