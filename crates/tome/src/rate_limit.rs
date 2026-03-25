//! Per-API rate limiting.
//!
//! Enforces limits:
//! - arXiv: 1 request per 3 seconds
//! - Semantic Scholar: 100 requests per 5 minutes
//! - CrossRef: 50 requests per second
//! - OpenAlex: 10 requests per second (polite pool)
//! - HN Algolia: 10,000 requests per hour

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Rate limiter configuration for a single API.
#[derive(Debug, Clone)]
pub struct RateLimit {
    /// Maximum requests allowed in the window.
    pub max_requests: u32,
    /// Time window for the limit.
    pub window: Duration,
}

/// Tracks request timestamps per API for rate limiting.
pub struct RateLimiter {
    limits: HashMap<String, RateLimit>,
    history: Mutex<HashMap<String, Vec<Instant>>>,
}

impl RateLimiter {
    /// Creates a new rate limiter with default API limits.
    pub fn new() -> Self {
        let mut limits = HashMap::new();
        limits.insert(
            "arxiv".to_string(),
            RateLimit {
                max_requests: 1,
                window: Duration::from_secs(3),
            },
        );
        limits.insert(
            "semantic_scholar".to_string(),
            RateLimit {
                max_requests: 100,
                window: Duration::from_secs(300),
            },
        );
        limits.insert(
            "crossref".to_string(),
            RateLimit {
                max_requests: 50,
                window: Duration::from_secs(1),
            },
        );
        limits.insert(
            "openalex".to_string(),
            RateLimit {
                max_requests: 10,
                window: Duration::from_secs(1),
            },
        );
        limits.insert(
            "hn_algolia".to_string(),
            RateLimit {
                max_requests: 10_000,
                window: Duration::from_secs(3600),
            },
        );

        Self {
            limits,
            history: Mutex::new(HashMap::new()),
        }
    }

    /// Check if a request to the given API is allowed. Returns Ok(()) or
    /// Err with the Duration to wait.
    pub fn check(&self, api: &str) -> Result<(), Duration> {
        let Some(limit) = self.limits.get(api) else {
            return Ok(()); // No limit configured
        };

        let mut history = self.history.lock().unwrap();
        let timestamps = history.entry(api.to_string()).or_default();

        let now = Instant::now();
        let window_start = now - limit.window;

        // Remove expired entries
        timestamps.retain(|t| *t > window_start);

        if timestamps.len() >= limit.max_requests as usize {
            let oldest = timestamps[0];
            let wait = limit.window - (now - oldest);
            return Err(wait);
        }

        timestamps.push(now);
        Ok(())
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_first_request() {
        let limiter = RateLimiter::new();
        assert!(limiter.check("arxiv").is_ok());
    }

    #[test]
    fn blocks_over_limit() {
        let limiter = RateLimiter::new();
        // arXiv: 1 req / 3s
        assert!(limiter.check("arxiv").is_ok());
        assert!(limiter.check("arxiv").is_err());
    }

    #[test]
    fn unknown_api_always_allowed() {
        let limiter = RateLimiter::new();
        for _ in 0..100 {
            assert!(limiter.check("unknown_api").is_ok());
        }
    }
}
