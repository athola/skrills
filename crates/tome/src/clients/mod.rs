//! API client modules for external research services.

pub mod arxiv;
pub mod crossref;
pub mod hn_algolia;
pub mod openalex;
pub mod semantic_scholar;
pub mod unpaywall;

use crate::{TomeError, TomeResult};
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::StatusCode;
use std::time::Duration;

/// Per-request timeout shared by every client.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Largest response body any client will buffer. The APIs return at most a
/// few hundred results per page, well under this.
pub(crate) const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Back-off used for an HTTP 429 whose `Retry-After` is absent or is an
/// HTTP-date rather than a number of seconds.
pub(crate) const DEFAULT_RETRY_AFTER_SECS: u64 = 60;

/// Build the HTTP client for `api`.
///
/// If the configured builder fails, retry with the timeout alone so a
/// fallback client never loses it. `reqwest::Client::new()` is the last
/// resort only in name: it panics on the same TLS-backend failure that
/// makes the timeout-only builder fail.
pub(crate) fn http_client(api: &str, user_agent: Option<&str>) -> reqwest::Client {
    let mut builder = reqwest::Client::builder().timeout(REQUEST_TIMEOUT);
    if let Some(ua) = user_agent {
        builder = builder.user_agent(ua);
    }
    builder.build().unwrap_or_else(|e| {
        tracing::warn!(api, error = %e, "client builder failed, retrying with timeout only");
        reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Map a non-success status to an error: 429 becomes `RateLimited` so
/// callers can back off, everything else `Api`.
pub(crate) fn status_error(
    api: &str,
    status: StatusCode,
    headers: &HeaderMap,
    context: &str,
) -> TomeError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return TomeError::RateLimited {
            retry_after_secs: retry_after_secs(headers),
        };
    }
    TomeError::Api {
        api: api.to_string(),
        message: format!("HTTP {status}{context}"),
    }
}

/// Return `Ok(resp)` for a success status, otherwise the mapped error.
pub(crate) fn ensure_success(
    api: &str,
    resp: reqwest::Response,
    context: &str,
) -> TomeResult<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        Ok(resp)
    } else {
        Err(status_error(api, status, resp.headers(), context))
    }
}

fn retry_after_secs(headers: &HeaderMap) -> u64 {
    headers
        .get(RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(DEFAULT_RETRY_AFTER_SECS)
}

/// Read the body, failing once it passes `MAX_RESPONSE_BYTES` instead of
/// buffering an unbounded response.
pub(crate) async fn read_body(api: &str, mut resp: reqwest::Response) -> TomeResult<Vec<u8>> {
    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BYTES as u64 {
            return Err(too_large(api, MAX_RESPONSE_BYTES));
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        append_capped(api, &mut body, &chunk, MAX_RESPONSE_BYTES)?;
    }
    Ok(body)
}

/// Read a capped body and parse it as JSON.
pub(crate) async fn read_json(api: &str, resp: reqwest::Response) -> TomeResult<serde_json::Value> {
    Ok(serde_json::from_slice(&read_body(api, resp).await?)?)
}

fn append_capped(api: &str, body: &mut Vec<u8>, chunk: &[u8], max: usize) -> TomeResult<()> {
    if body.len().saturating_add(chunk.len()) > max {
        return Err(too_large(api, max));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

fn too_large(api: &str, max: usize) -> TomeError {
    TomeError::Api {
        api: api.to_string(),
        message: format!("response larger than {max} bytes"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn too_many_requests_maps_to_rate_limited_with_retry_after() {
        let mut headers = HeaderMap::new();
        headers.insert(RETRY_AFTER, HeaderValue::from_static("17"));
        let err = status_error("s2", StatusCode::TOO_MANY_REQUESTS, &headers, "");
        assert!(
            matches!(
                err,
                TomeError::RateLimited {
                    retry_after_secs: 17
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn rate_limit_without_numeric_retry_after_uses_the_default() {
        let mut headers = HeaderMap::new();
        let err = status_error("s2", StatusCode::TOO_MANY_REQUESTS, &headers, "");
        assert!(matches!(
            err,
            TomeError::RateLimited { retry_after_secs } if retry_after_secs == DEFAULT_RETRY_AFTER_SECS
        ));
        headers.insert(
            RETRY_AFTER,
            HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        let err = status_error("s2", StatusCode::TOO_MANY_REQUESTS, &headers, "");
        assert!(matches!(
            err,
            TomeError::RateLimited { retry_after_secs } if retry_after_secs == DEFAULT_RETRY_AFTER_SECS
        ));
    }

    #[test]
    fn other_failures_stay_api_errors_with_context() {
        let err = status_error(
            "crossref",
            StatusCode::NOT_FOUND,
            &HeaderMap::new(),
            " for DOI x",
        );
        match err {
            TomeError::Api { api, message } => {
                assert_eq!(api, "crossref");
                assert_eq!(message, "HTTP 404 Not Found for DOI x");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn body_is_refused_once_it_passes_the_cap() {
        let mut body = Vec::new();
        append_capped("a", &mut body, b"1234", 6).unwrap();
        append_capped("a", &mut body, b"56", 6).unwrap();
        assert_eq!(body, b"123456");
        assert!(append_capped("a", &mut body, b"7", 6).is_err());
        assert_eq!(body.len(), 6, "rejected chunk is not appended");
    }
}
