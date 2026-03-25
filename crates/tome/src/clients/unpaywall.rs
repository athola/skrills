//! Unpaywall API client for open-access PDF lookup.
//! API: https://api.unpaywall.org

use crate::TomeResult;

const BASE_URL: &str = "https://api.unpaywall.org/v2";
const EMAIL: &str = "research@skrills.dev";

pub struct UnpaywallClient {
    http: reqwest::Client,
}

impl UnpaywallClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Look up the open-access PDF URL for a DOI.
    pub async fn find_pdf_url(&self, doi: &str) -> TomeResult<Option<String>> {
        let resp = self
            .http
            .get(&format!("{BASE_URL}/{doi}"))
            .query(&[("email", EMAIL)])
            .send()
            .await?;

        if !resp.status().is_success() {
            return Ok(None); // Not found is not an error
        }

        let body: serde_json::Value = resp.json().await?;
        Ok(body["best_oa_location"]["url_for_pdf"]
            .as_str()
            .map(String::from))
    }
}
