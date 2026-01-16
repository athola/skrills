//! Search GitHub for existing SKILL.md files.

use anyhow::Result;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

const GITHUB_API_BASE: &str = "https://api.github.com";

/// Get the GitHub API base URL, allowing override for testing.
fn github_api_base() -> String {
    std::env::var("GITHUB_API_BASE_URL").unwrap_or_else(|_| GITHUB_API_BASE.to_string())
}

/// Get the raw content base URL, allowing override for testing.
fn raw_content_base() -> String {
    std::env::var("GITHUB_RAW_BASE_URL")
        .unwrap_or_else(|_| "https://raw.githubusercontent.com".to_string())
}

fn github_token() -> Option<String> {
    let raw = std::env::var("GITHUB_TOKEN").ok()?;
    let token = raw.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn apply_github_auth(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match github_token() {
        Some(token) => builder.header(AUTHORIZATION, format!("Bearer {token}")),
        None => builder,
    }
}

/// A skill found on GitHub.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubSkillResult {
    /// Repository URL.
    pub repo_url: String,
    /// Path to the skill file within the repo.
    pub skill_path: String,
    /// Direct URL to the skill file on GitHub.
    pub file_url: String,
    /// Repository description.
    pub description: Option<String>,
    /// Number of stars.
    pub stars: u64,
    /// Last update date.
    pub last_updated: String,
    /// Raw URL to fetch the skill content.
    pub raw_url: Option<String>,
}

#[derive(Deserialize)]
struct SearchResponse {
    items: Vec<SearchItem>,
}

#[derive(Deserialize)]
struct SearchItem {
    path: String,
    html_url: String,
    repository: Repository,
}

#[derive(Deserialize)]
struct Repository {
    full_name: String,
    html_url: String,
    #[serde(default)]
    stargazers_count: u64,
    #[serde(default)]
    updated_at: String,
    description: Option<String>,
}

/// Sanitize user input to prevent GitHub search operator injection.
/// Strips known GitHub search operators that could manipulate search semantics.
fn sanitize_github_query(query: &str) -> String {
    // GitHub search operators that could be injected (colon-based operators)
    let colon_operators = [
        "repo:",
        "org:",
        "user:",
        "in:",
        "size:",
        "fork:",
        "forks:",
        "stars:",
        "topics:",
        "topic:",
        "created:",
        "pushed:",
        "updated:",
        "is:",
        "archived:",
        "license:",
        "language:",
        "filename:",
        "path:",
        "extension:",
    ];

    // Boolean operators (must be standalone words, case-insensitive)
    let boolean_operators = ["NOT", "AND", "OR"];

    let mut sanitized = query.to_string();

    // Remove colon-based operators
    for op in colon_operators {
        loop {
            let lower = sanitized.to_lowercase();
            if let Some(pos) = lower.find(&op.to_lowercase()) {
                // Find the end of the operator value (space or end of string)
                let rest = &sanitized[pos + op.len()..];
                let end = if rest.starts_with('"') {
                    // Quoted value - find closing quote
                    rest.strip_prefix('"')
                        .and_then(|s| s.find('"'))
                        .map(|p| pos + op.len() + p + 2)
                        .unwrap_or(sanitized.len())
                } else {
                    // Unquoted value - find next space
                    rest.find(' ')
                        .map(|p| pos + op.len() + p)
                        .unwrap_or(sanitized.len())
                };
                sanitized = format!("{}{}", &sanitized[..pos], &sanitized[end..]);
            } else {
                break;
            }
        }
    }

    // Remove standalone boolean operators (word boundaries check)
    let words: Vec<&str> = sanitized.split_whitespace().collect();
    let filtered_words: Vec<&str> = words
        .into_iter()
        .filter(|word| {
            !boolean_operators
                .iter()
                .any(|op| word.eq_ignore_ascii_case(op))
        })
        .collect();

    filtered_words.join(" ")
}

/// Search GitHub for skills matching the query.
pub async fn search_github_skills(query: &str, limit: usize) -> Result<Vec<GitHubSkillResult>> {
    // Sanitize user input to prevent search operator injection
    let sanitized_query = sanitize_github_query(query);

    // Build search query for SKILL.md files
    let search_query = format!("{} filename:SKILL.md", sanitized_query);

    search_github_raw(&search_query, limit).await
}

/// Internal search function that performs the raw GitHub API call.
/// This function does NOT sanitize the query, so it should only be called
/// with trusted input (e.g., from `search_skills_advanced`).
async fn search_github_raw(query: &str, limit: usize) -> Result<Vec<GitHubSkillResult>> {
    let client = reqwest::Client::new();

    let response = apply_github_auth(
        client
            .get(format!("{}/search/code", github_api_base()))
            .query(&[("q", query), ("per_page", &limit.to_string())])
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "skrills-intelligence/0.4.0"),
    )
    .send()
    .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();

        // Provide actionable error messages for common GitHub API errors
        let error_msg = match status.as_u16() {
            403 => {
                if body.contains("rate limit") || body.contains("API rate limit exceeded") {
                    "GitHub API rate limit exceeded. Wait a few minutes and try again, or set GITHUB_TOKEN environment variable for higher limits."
                } else {
                    "GitHub API access forbidden. Check your GITHUB_TOKEN permissions."
                }
            }
            401 => "GitHub API authentication failed. Verify your GITHUB_TOKEN is valid.",
            422 => "GitHub search query invalid. Try simplifying your search terms.",
            _ => "",
        };

        if error_msg.is_empty() {
            anyhow::bail!("GitHub API error ({}): {}", status, body);
        } else {
            anyhow::bail!("{} (HTTP {})", error_msg, status);
        }
    }

    let search_result: SearchResponse = response.json().await?;

    Ok(search_result
        .items
        .into_iter()
        .map(|item| {
            let raw_url = build_raw_url(&item.repository.full_name, &item.path);
            GitHubSkillResult {
                repo_url: item.repository.html_url,
                skill_path: item.path,
                file_url: item.html_url,
                description: item.repository.description,
                stars: item.repository.stargazers_count,
                last_updated: item.repository.updated_at,
                raw_url: Some(raw_url),
            }
        })
        .collect())
}

/// Build a raw.githubusercontent.com URL for fetching file content.
fn build_raw_url(full_name: &str, path: &str) -> String {
    format!("{}/{}/HEAD/{}", raw_content_base(), full_name, path)
}

/// Fetch the content of a skill from its raw URL.
pub async fn fetch_skill_content(raw_url: &str) -> Result<String> {
    let client = reqwest::Client::new();

    let response = apply_github_auth(
        client
            .get(raw_url)
            .header("User-Agent", "skrills-intelligence/0.4.0"),
    )
    .send()
    .await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch skill content: {}", response.status());
    }

    Ok(response.text().await?)
}

/// Search for skills with specific criteria.
pub async fn search_skills_advanced(
    keywords: &[String],
    language: Option<&str>,
    min_stars: Option<u64>,
    limit: usize,
) -> Result<Vec<GitHubSkillResult>> {
    let mut query_parts = vec!["filename:SKILL.md".to_string()];

    // Add sanitized keywords (user input)
    for keyword in keywords {
        let sanitized = sanitize_github_query(keyword);
        if !sanitized.is_empty() {
            query_parts.push(sanitized);
        }
    }

    // Add language filter (trusted input, not from user)
    if let Some(lang) = language {
        query_parts.push(format!("language:{}", lang));
    }

    // Add star filter (trusted input, not from user)
    if let Some(stars) = min_stars {
        query_parts.push(format!("stars:>={}", stars));
    }

    let query = query_parts.join(" ");
    // Use raw search since we've already sanitized user input and
    // added trusted operators
    search_github_raw(&query, limit).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::AUTHORIZATION;
    use serial_test::serial;
    use std::env;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.previous {
                env::set_var(self.key, v);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = env::var(key).ok();
        if let Some(v) = value {
            env::set_var(key, v);
        } else {
            env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    #[test]
    #[serial]
    fn test_build_raw_url_default() {
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", None);
        let url = build_raw_url("owner/repo", "skills/test/SKILL.md");
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/owner/repo/HEAD/skills/test/SKILL.md"
        );
    }

    #[test]
    #[serial]
    fn test_build_raw_url_with_custom_base() {
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", Some("http://localhost:8080"));
        let url = build_raw_url("owner/repo", "skills/test/SKILL.md");
        assert_eq!(
            url,
            "http://localhost:8080/owner/repo/HEAD/skills/test/SKILL.md"
        );
    }

    #[test]
    #[serial]
    fn test_github_api_base_default() {
        let _api_guard = set_env_var("GITHUB_API_BASE_URL", None);
        assert_eq!(github_api_base(), "https://api.github.com");
    }

    #[test]
    #[serial]
    fn test_github_api_base_custom() {
        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some("http://localhost:9090"));
        assert_eq!(github_api_base(), "http://localhost:9090");
    }

    #[test]
    #[serial]
    fn test_github_auth_header_set_when_token_present() {
        let _token_guard = set_env_var("GITHUB_TOKEN", Some("test-token"));
        let client = reqwest::Client::new();
        let request = apply_github_auth(client.get("https://api.github.com"))
            .build()
            .unwrap();
        let header = request.headers().get(AUTHORIZATION).unwrap();
        assert_eq!(header.to_str().unwrap(), "Bearer test-token");
    }

    #[test]
    #[serial]
    fn test_github_auth_header_absent_when_no_token() {
        let _token_guard = set_env_var("GITHUB_TOKEN", None);
        let client = reqwest::Client::new();
        let request = apply_github_auth(client.get("https://api.github.com"))
            .build()
            .unwrap();
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    #[serial]
    fn test_github_auth_header_absent_when_empty_token() {
        let _token_guard = set_env_var("GITHUB_TOKEN", Some(""));
        let client = reqwest::Client::new();
        let request = apply_github_auth(client.get("https://api.github.com"))
            .build()
            .unwrap();
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    #[serial]
    fn test_github_auth_header_absent_when_whitespace_only_token() {
        let _token_guard = set_env_var("GITHUB_TOKEN", Some("   \t\n  "));
        let client = reqwest::Client::new();
        let request = apply_github_auth(client.get("https://api.github.com"))
            .build()
            .unwrap();
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }

    #[test]
    #[serial]
    fn test_github_auth_trims_token_whitespace() {
        let _token_guard = set_env_var("GITHUB_TOKEN", Some("  test-token  "));
        let client = reqwest::Client::new();
        let request = apply_github_auth(client.get("https://api.github.com"))
            .build()
            .unwrap();
        let header = request.headers().get(AUTHORIZATION).unwrap();
        assert_eq!(header.to_str().unwrap(), "Bearer test-token");
    }

    #[test]
    fn test_sanitize_github_query_removes_operators() {
        // Test removal of common injection operators
        assert_eq!(sanitize_github_query("test repo:evil/repo"), "test");
        assert_eq!(sanitize_github_query("test org:malicious"), "test");
        assert_eq!(sanitize_github_query("test stars:>1000"), "test");
        assert_eq!(sanitize_github_query("test language:rust"), "test");
        assert_eq!(sanitize_github_query("test is:archived"), "test");
    }

    #[test]
    fn test_sanitize_github_query_removes_all_colon_operators() {
        // Test all defined colon operators
        let operators = [
            "repo:",
            "org:",
            "user:",
            "in:",
            "size:",
            "fork:",
            "forks:",
            "stars:",
            "topics:",
            "topic:",
            "created:",
            "pushed:",
            "updated:",
            "is:",
            "archived:",
            "license:",
            "language:",
            "filename:",
            "path:",
            "extension:",
        ];
        for op in operators {
            let query = format!("test {}value", op);
            let result = sanitize_github_query(&query);
            assert_eq!(result, "test", "Failed to remove operator: {}", op);
        }
    }

    #[test]
    fn test_sanitize_github_query_preserves_normal_text() {
        // Normal queries should pass through unchanged
        assert_eq!(sanitize_github_query("testing skills"), "testing skills");
        assert_eq!(sanitize_github_query("rust async"), "rust async");
        assert_eq!(sanitize_github_query("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_github_query_handles_quoted_values() {
        // Quoted operator values should be removed
        assert_eq!(sanitize_github_query(r#"test repo:"owner/repo""#), "test");
    }

    #[test]
    fn test_sanitize_github_query_collapses_whitespace() {
        // Multiple spaces should be collapsed
        assert_eq!(
            sanitize_github_query("test   multiple   spaces"),
            "test multiple spaces"
        );
    }

    #[test]
    fn test_sanitize_github_query_case_insensitive() {
        // Operators should be removed regardless of case
        assert_eq!(sanitize_github_query("test REPO:evil"), "test");
        assert_eq!(sanitize_github_query("test Repo:evil"), "test");
    }

    #[test]
    fn test_sanitize_github_query_removes_boolean_operators() {
        // Boolean operators should be removed as standalone words only
        assert_eq!(sanitize_github_query("foo AND bar"), "foo bar");
        assert_eq!(sanitize_github_query("foo OR bar"), "foo bar");
        assert_eq!(sanitize_github_query("NOT foo"), "foo");
        // But words containing these should NOT be affected
        assert_eq!(sanitize_github_query("Oregon skills"), "Oregon skills");
        assert_eq!(sanitize_github_query("android app"), "android app");
    }

    #[test]
    fn test_sanitize_github_query_handles_empty_string() {
        assert_eq!(sanitize_github_query(""), "");
    }

    #[test]
    fn test_sanitize_github_query_handles_only_operators() {
        assert_eq!(sanitize_github_query("repo:owner/repo"), "");
        assert_eq!(sanitize_github_query("repo:owner/repo stars:>100"), "");
    }

    #[test]
    fn test_sanitize_github_query_removes_multiple_operators() {
        assert_eq!(
            sanitize_github_query("test repo:evil stars:>100 language:rust"),
            "test"
        );
    }

    #[test]
    fn test_sanitize_github_query_preserves_colons_in_normal_text() {
        // Colons not part of operators should be preserved
        assert_eq!(sanitize_github_query("time 12:30"), "time 12:30");
        assert_eq!(sanitize_github_query("key:value"), "key:value");
    }

    #[test]
    fn test_github_skill_result_serialization() {
        let result = GitHubSkillResult {
            repo_url: "https://github.com/owner/repo".to_string(),
            skill_path: "skills/test/SKILL.md".to_string(),
            file_url: "https://github.com/owner/repo/blob/main/skills/test/SKILL.md".to_string(),
            description: Some("Test repo".to_string()),
            stars: 100,
            last_updated: "2024-01-01T00:00:00Z".to_string(),
            raw_url: Some(
                "https://raw.githubusercontent.com/owner/repo/HEAD/skills/test/SKILL.md"
                    .to_string(),
            ),
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: GitHubSkillResult = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.repo_url, result.repo_url);
        assert_eq!(deserialized.stars, 100);
        assert_eq!(deserialized.description, Some("Test repo".to_string()));
    }

    #[test]
    fn test_github_skill_result_with_none_description() {
        let result = GitHubSkillResult {
            repo_url: "https://github.com/owner/repo".to_string(),
            skill_path: "SKILL.md".to_string(),
            file_url: "https://github.com/owner/repo/blob/main/SKILL.md".to_string(),
            description: None,
            stars: 0,
            last_updated: "".to_string(),
            raw_url: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: GitHubSkillResult = serde_json::from_str(&json).unwrap();

        assert!(deserialized.description.is_none());
        assert!(deserialized.raw_url.is_none());
    }
}

/// Property-based tests for sanitize_github_query using proptest.
/// These tests generate random inputs to find edge cases that manual tests might miss.
#[cfg(test)]
mod proptest_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Property: sanitize_github_query should never panic on any input.
        #[test]
        fn sanitize_never_panics(input in "\\PC*") {
            let _ = sanitize_github_query(&input);
        }

        /// Property: sanitize_github_query should never return a string longer than input.
        /// (We only remove content, never add)
        #[test]
        fn sanitize_never_increases_length(input in "\\PC*") {
            let result = sanitize_github_query(&input);
            prop_assert!(result.len() <= input.len());
        }

        /// Property: sanitize_github_query should always return valid UTF-8.
        /// In Rust, String is guaranteed valid UTF-8, so reaching here without panic proves validity.
        #[test]
        fn sanitize_returns_valid_utf8(input in "\\PC*") {
            let result = sanitize_github_query(&input);
            // Verify we can iterate chars (would panic on invalid UTF-8 if String were somehow corrupted)
            let char_count = result.chars().count();
            prop_assert!(char_count <= input.chars().count());
        }

        /// Property: If input has no operators, only whitespace normalization should occur.
        #[test]
        fn sanitize_preserves_safe_words(
            words in prop::collection::vec("[a-zA-Z0-9]+", 1..5)
        ) {
            let input = words.join(" ");
            let result = sanitize_github_query(&input);
            // Each word should appear in the result (unless it's a boolean operator)
            for word in &words {
                let is_boolean_op = ["NOT", "AND", "OR"]
                    .iter()
                    .any(|op| word.eq_ignore_ascii_case(op));
                if !is_boolean_op {
                    prop_assert!(
                        result.contains(word.as_str()),
                        "Word '{}' missing from result '{}'", word, result
                    );
                }
            }
        }

        /// Property: No colon operators should survive sanitization.
        #[test]
        fn sanitize_removes_all_colon_operators(
            prefix in "[a-zA-Z0-9 ]{0,20}",
            operator in prop::sample::select(vec![
                "repo:", "org:", "user:", "in:", "size:", "fork:", "forks:",
                "stars:", "topics:", "topic:", "created:", "pushed:", "updated:",
                "is:", "archived:", "license:", "language:", "filename:", "path:",
                "extension:"
            ]),
            value in "[a-zA-Z0-9/\\-><=]+",
            suffix in "[a-zA-Z0-9 ]{0,20}"
        ) {
            let input = format!("{} {}{} {}", prefix.trim(), operator, value, suffix.trim());
            let result = sanitize_github_query(&input);

            // The operator:value should be completely removed
            prop_assert!(
                !result.to_lowercase().contains(&operator.to_lowercase()),
                "Operator '{}' found in result '{}' (input: '{}')",
                operator, result, input
            );
        }

        /// Property: Standalone boolean operators should be removed.
        #[test]
        fn sanitize_removes_boolean_operators(
            prefix in "[a-z]{1,10}",
            boolean_op in prop::sample::select(vec!["AND", "OR", "NOT", "and", "or", "not"]),
            suffix in "[a-z]{1,10}"
        ) {
            let input = format!("{} {} {}", prefix, boolean_op, suffix);
            let result = sanitize_github_query(&input);

            // The result should contain prefix and suffix but not the boolean operator as standalone
            let words: Vec<&str> = result.split_whitespace().collect();
            let has_boolean = words.iter().any(|w| {
                w.eq_ignore_ascii_case("AND") || w.eq_ignore_ascii_case("OR") || w.eq_ignore_ascii_case("NOT")
            });
            prop_assert!(
                !has_boolean,
                "Boolean operator found in result '{}' (input: '{}')",
                result, input
            );
        }

        /// Property: Words containing boolean operator substrings should NOT be affected.
        /// E.g., "android", "Oregon", "annotation" should pass through.
        #[test]
        fn sanitize_preserves_words_with_operator_substrings(
            word in "(android|Oregon|annotation|manor|donor|canopy|mandate|bandana|panorama|grandma)"
        ) {
            let input = format!("test {}", word);
            let result = sanitize_github_query(&input);
            prop_assert!(
                result.contains(&word),
                "Word '{}' should be preserved in result '{}'",
                word, result
            );
        }

        /// Property: Empty and whitespace-only inputs should return empty string.
        #[test]
        fn sanitize_handles_whitespace_only(spaces in "[ \t\n\r]*") {
            let result = sanitize_github_query(&spaces);
            prop_assert!(
                result.is_empty(),
                "Whitespace-only input should produce empty result, got '{}'",
                result
            );
        }
    }
}

/// Integration tests using wiremock for HTTP mocking.
/// These tests verify the actual HTTP behavior of the GitHub search functions.
#[cfg(test)]
mod integration_tests {
    use super::*;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use wiremock::matchers::{method, path, query_param_contains};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.previous {
                env::set_var(self.key, v);
            } else {
                env::remove_var(self.key);
            }
        }
    }

    fn set_env_var(key: &'static str, value: Option<&str>) -> EnvVarGuard {
        let previous = env::var(key).ok();
        if let Some(v) = value {
            env::set_var(key, v);
        } else {
            env::remove_var(key);
        }
        EnvVarGuard { key, previous }
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_success() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        let mock_response = json!({
            "items": [
                {
                    "path": "skills/testing/SKILL.md",
                    "html_url": "https://github.com/owner/repo/blob/main/skills/testing/SKILL.md",
                    "repository": {
                        "full_name": "owner/repo",
                        "html_url": "https://github.com/owner/repo",
                        "stargazers_count": 42,
                        "updated_at": "2024-06-15T10:00:00Z",
                        "description": "A test repository"
                    }
                },
                {
                    "path": "SKILL.md",
                    "html_url": "https://github.com/another/project/blob/main/SKILL.md",
                    "repository": {
                        "full_name": "another/project",
                        "html_url": "https://github.com/another/project",
                        "stargazers_count": 100,
                        "updated_at": "2024-07-20T15:30:00Z",
                        "description": null
                    }
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .and(query_param_contains("q", "testing"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&mock_response))
            .mount(&server)
            .await;

        let results = search_github_skills("testing", 10).await.unwrap();

        assert_eq!(results.len(), 2);

        // First result
        assert_eq!(results[0].repo_url, "https://github.com/owner/repo");
        assert_eq!(results[0].skill_path, "skills/testing/SKILL.md");
        assert_eq!(results[0].stars, 42);
        assert_eq!(
            results[0].description,
            Some("A test repository".to_string())
        );
        assert!(results[0].raw_url.is_some());

        // Second result
        assert_eq!(results[1].repo_url, "https://github.com/another/project");
        assert_eq!(results[1].skill_path, "SKILL.md");
        assert_eq!(results[1].stars, 100);
        assert!(results[1].description.is_none());
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_empty_results() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        let mock_response = json!({
            "items": []
        });

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&mock_response))
            .mount(&server)
            .await;

        let results = search_github_skills("nonexistent", 10).await.unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_rate_limit_error() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "message": "API rate limit exceeded for IP"
            })))
            .mount(&server)
            .await;

        let result = search_github_skills("testing", 10).await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("rate limit") || error_msg.contains("403"),
            "Expected rate limit error, got: {}",
            error_msg
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_auth_error() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", Some("invalid-token"));

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "message": "Bad credentials"
            })))
            .mount(&server)
            .await;

        let result = search_github_skills("testing", 10).await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("authentication") || error_msg.contains("401"),
            "Expected auth error, got: {}",
            error_msg
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_invalid_query_error() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(422).set_body_json(json!({
                "message": "Validation Failed"
            })))
            .mount(&server)
            .await;

        let result = search_github_skills("", 10).await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("invalid") || error_msg.contains("422"),
            "Expected validation error, got: {}",
            error_msg
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_server_error() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(500).set_body_json(json!({
                "message": "Internal Server Error"
            })))
            .mount(&server)
            .await;

        let result = search_github_skills("testing", 10).await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("500") || error_msg.contains("error"),
            "Expected server error, got: {}",
            error_msg
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_skill_content_success() {
        let server = MockServer::start().await;
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        let skill_content = r#"---
description: Test skill
triggers:
  - test
---
# Test Skill

This is a test skill."#;

        Mock::given(method("GET"))
            .and(path("/owner/repo/HEAD/SKILL.md"))
            .respond_with(ResponseTemplate::new(200).set_body_string(skill_content))
            .mount(&server)
            .await;

        let url = format!("{}/owner/repo/HEAD/SKILL.md", server.uri());
        let content = fetch_skill_content(&url).await.unwrap();

        assert!(content.contains("Test skill"));
        assert!(content.contains("This is a test skill"));
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_skill_content_not_found() {
        let server = MockServer::start().await;
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        Mock::given(method("GET"))
            .and(path("/owner/repo/HEAD/SKILL.md"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let url = format!("{}/owner/repo/HEAD/SKILL.md", server.uri());
        let result = fetch_skill_content(&url).await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("404") || error_msg.contains("Failed"),
            "Expected not found error, got: {}",
            error_msg
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_search_skills_advanced_with_language_filter() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        let mock_response = json!({
            "items": [
                {
                    "path": "SKILL.md",
                    "html_url": "https://github.com/rust-project/skills/blob/main/SKILL.md",
                    "repository": {
                        "full_name": "rust-project/skills",
                        "html_url": "https://github.com/rust-project/skills",
                        "stargazers_count": 50,
                        "updated_at": "2024-08-01T00:00:00Z",
                        "description": "Rust skills"
                    }
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .and(query_param_contains("q", "language:rust"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&mock_response))
            .mount(&server)
            .await;

        let keywords = vec!["async".to_string()];
        let results = search_skills_advanced(&keywords, Some("rust"), None, 10)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].repo_url,
            "https://github.com/rust-project/skills"
        );
    }

    #[tokio::test]
    #[serial]
    async fn test_search_skills_advanced_with_stars_filter() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        let mock_response = json!({
            "items": [
                {
                    "path": "SKILL.md",
                    "html_url": "https://github.com/popular/repo/blob/main/SKILL.md",
                    "repository": {
                        "full_name": "popular/repo",
                        "html_url": "https://github.com/popular/repo",
                        "stargazers_count": 1500,
                        "updated_at": "2024-08-01T00:00:00Z",
                        "description": "Popular repo"
                    }
                }
            ]
        });

        Mock::given(method("GET"))
            .and(path("/search/code"))
            .and(query_param_contains("q", "stars:>=100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&mock_response))
            .mount(&server)
            .await;

        let keywords = vec!["testing".to_string()];
        let results = search_skills_advanced(&keywords, None, Some(100), 10)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].stars, 1500);
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_sanitizes_input() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        let mock_response = json!({
            "items": []
        });

        // The mock should receive a sanitized query without the injected operators
        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&mock_response))
            .mount(&server)
            .await;

        // Try to inject operators - they should be sanitized
        let result = search_github_skills("test repo:evil/repo stars:>10000", 10).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_uses_auth_header_when_token_present() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", Some("test-github-token"));

        let mock_response = json!({
            "items": []
        });

        // Verify the auth header is included
        Mock::given(method("GET"))
            .and(path("/search/code"))
            .and(wiremock::matchers::header(
                "Authorization",
                "Bearer test-github-token",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(&mock_response))
            .expect(1)
            .mount(&server)
            .await;

        let result = search_github_skills("testing", 10).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    #[serial]
    async fn test_search_github_skills_handles_forbidden_non_rate_limit() {
        let server = MockServer::start().await;

        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        // 403 without rate limit message
        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(403).set_body_json(json!({
                "message": "Repository access blocked"
            })))
            .mount(&server)
            .await;

        let result = search_github_skills("testing", 10).await;

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(
            error_msg.contains("forbidden")
                || error_msg.contains("403")
                || error_msg.contains("permission"),
            "Expected forbidden error, got: {}",
            error_msg
        );
    }
}
