//! Unpaywall API client for open-access PDF lookup.
//! API: <https://api.unpaywall.org>

use crate::TomeResult;

const BASE_URL: &str = "https://api.unpaywall.org/v2";
const DEFAULT_EMAIL: &str = "research@skrills.dev";

pub struct UnpaywallClient {
    http: reqwest::Client,
    email: String,
}

impl Default for UnpaywallClient {
    fn default() -> Self {
        Self::new(DEFAULT_EMAIL.to_string())
    }
}

impl UnpaywallClient {
    pub fn new(email: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            email,
        }
    }

    /// Look up the open-access PDF URL for a DOI.
    pub async fn find_pdf_url(&self, doi: &str) -> TomeResult<Option<String>> {
        let resp = self
            .http
            .get(format!(
                "{BASE_URL}/{}",
                url::form_urlencoded::byte_serialize(doi.as_bytes()).collect::<String>()
            ))
            .query(&[("email", &self.email)])
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            return Err(crate::TomeError::Api {
                api: "unpaywall".to_string(),
                message: format!("HTTP {status}"),
            });
        }

        let body: serde_json::Value = resp.json().await?;
        Ok(body["best_oa_location"]["url_for_pdf"]
            .as_str()
            .map(String::from))
    }
}
