//! Search GitHub for existing SKILL.md files.

use anyhow::Result;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const GITHUB_API_BASE: &str = "https://api.github.com";
const GITHUB_RAW_BASE: &str = "https://raw.githubusercontent.com";

/// Whole-request timeout for GitHub calls.
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Connect timeout for GitHub calls.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest response body read from GitHub. Search pages and SKILL.md files
/// are far smaller; anything bigger is refused rather than buffered.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// How much of an error body is echoed into the error message.
const MAX_ERROR_BODY_CHARS: usize = 2048;

/// Read a base-URL override from the environment.
///
/// Only test builds honour it. In a release build a stray
/// `GITHUB_API_BASE_URL` would otherwise redirect every search, with the
/// `GITHUB_TOKEN` bearer header, to an arbitrary (possibly plain-http) host.
#[cfg(test)]
fn base_url_override(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

#[cfg(not(test))]
fn base_url_override(_var: &str) -> Option<String> {
    None
}

/// The GitHub API base URL (overridable in tests only).
fn github_api_base() -> String {
    base_url_override("GITHUB_API_BASE_URL").unwrap_or_else(|| GITHUB_API_BASE.to_string())
}

/// The raw content base URL (overridable in tests only).
fn raw_content_base() -> String {
    base_url_override("GITHUB_RAW_BASE_URL").unwrap_or_else(|| GITHUB_RAW_BASE.to_string())
}

/// A client with request and connect timeouts.
fn build_client(timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(HTTP_CONNECT_TIMEOUT.min(timeout))
        .user_agent(concat!("skrills-intelligence/", env!("CARGO_PKG_VERSION")))
        .build()?)
}

/// Read a response body, refusing it once it grows past `max_bytes`.
async fn read_body_capped(mut response: reqwest::Response, max_bytes: usize) -> Result<String> {
    if let Some(len) = response.content_length() {
        if len > max_bytes as u64 {
            anyhow::bail!("response body of {len} bytes exceeds the {max_bytes}-byte limit");
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > max_bytes {
            anyhow::bail!("response body exceeds the {max_bytes}-byte limit");
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Shorten an error body before it is echoed into a message.
fn truncate_for_message(body: &str) -> String {
    if body.chars().count() <= MAX_ERROR_BODY_CHARS {
        body.to_string()
    } else {
        let head: String = body.chars().take(MAX_ERROR_BODY_CHARS).collect();
        format!("{head}... (truncated)")
    }
}

/// True when `url` has the same scheme, host and port as `base`.
fn same_origin(url: &reqwest::Url, base: &str) -> bool {
    match reqwest::Url::parse(base) {
        Ok(base) => {
            url.scheme() == base.scheme()
                && url.host_str() == base.host_str()
                && url.port_or_known_default() == base.port_or_known_default()
        }
        Err(_) => false,
    }
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

/// Byte offset of the first case-insensitive occurrence of `needle`.
///
/// `to_ascii_lowercase` is used rather than `to_lowercase` because only the
/// former preserves byte length ('\u{130}' is 2 bytes and lowercases to 3), and
/// the offset is used to slice the original string. Every operator searched for
/// here is ASCII, so folding only ASCII loses no match.
///
/// Only matches that start a token count: at the start of the string, or after
/// whitespace, `(` or a `-` negation. `plugin:foo` is ordinary text to GitHub,
/// so the `in:` inside it must not be stripped.
fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    let lowered = haystack.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    lowered
        .match_indices(&needle.to_ascii_lowercase())
        .map(|(pos, _)| pos)
        .find(|&pos| {
            let opens_token =
                |p: usize| p == 0 || matches!(bytes[p - 1], b'(' | b' ' | b'\t' | b'\n' | b'\r');
            // `-repo:x` negates a qualifier; `built-in:` is a plain word.
            opens_token(pos) || (bytes[pos - 1] == b'-' && opens_token(pos - 1))
        })
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
        while let Some(pos) = find_ignore_ascii_case(&sanitized, op) {
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
    let client = build_client(HTTP_TIMEOUT)?;

    let response = apply_github_auth(
        client
            .get(format!("{}/search/code", github_api_base()))
            .query(&[("q", query), ("per_page", &limit.to_string())])
            .header("Accept", "application/vnd.github.v3+json"),
    )
    .send()
    .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = read_body_capped(response, MAX_BODY_BYTES)
            .await
            .map(|b| truncate_for_message(&b))
            .unwrap_or_default();

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

        // Construct via the structured `IntelligenceError::GitHubApi`
        // variant so callers can match on `status` to decide whether
        // to retry, fall back, or surface to the user. The `?`
        // operator promotes it into anyhow::Error transparently.
        let message = if error_msg.is_empty() {
            body
        } else {
            format!("{error_msg} (body: {body})")
        };
        return Err(crate::IntelligenceError::GitHubApi {
            status: status.as_u16(),
            message,
        }
        .into());
    }

    let body = read_body_capped(response, MAX_BODY_BYTES).await?;
    let search_result: SearchResponse = serde_json::from_str(&body)?;

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
                raw_url,
            }
        })
        .collect())
}

/// Build a raw.githubusercontent.com URL for fetching file content.
///
/// Each path segment is percent-encoded, so a repository path containing a
/// space, `#` or `?` still names that file. Returns `None` if the base URL
/// cannot carry a path.
fn build_raw_url(full_name: &str, path: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(&raw_content_base()).ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.pop_if_empty();
        segments.extend(full_name.split('/').filter(|s| !s.is_empty()));
        segments.push("HEAD");
        segments.extend(path.split('/').filter(|s| !s.is_empty()));
    }
    Some(url.into())
}

/// Fetch the content of a skill from its raw URL.
///
/// Only URLs on the raw content host (`https://raw.githubusercontent.com`)
/// are fetched. Anything else is refused before a request is made, so the
/// `GITHUB_TOKEN` bearer header is never sent to another host.
pub async fn fetch_skill_content(raw_url: &str) -> Result<String> {
    let url = reqwest::Url::parse(raw_url)?;
    let raw_base = raw_content_base();
    if !same_origin(&url, &raw_base) {
        anyhow::bail!("refusing to fetch {raw_url}: only {raw_base} URLs are allowed");
    }

    let client = build_client(HTTP_TIMEOUT)?;
    let response = apply_github_auth(client.get(url)).send().await?;

    if !response.status().is_success() {
        return Err(crate::IntelligenceError::FetchFailed {
            status: response.status().as_u16(),
        }
        .into());
    }

    read_body_capped(response, MAX_BODY_BYTES).await
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

    /// `to_lowercase()` is not byte-length preserving: 'İ' (U+0130, 2 bytes)
    /// lowercases to 3 bytes. The operator search ran over the lowercased copy
    /// and sliced the original with the offset it found there, so a query
    /// carrying such a character panicked on an out-of-bounds slice. Reachable
    /// from the `search-skills-github` MCP tool directly, and from
    /// `create-skill`, which builds the query out of an unvalidated description.
    #[test]
    fn sanitize_github_query_handles_multibyte_case_change() {
        // Glued to a preceding word the operator is plain text to GitHub
        // (IN-22), so it is kept; what matters here is that nothing panics.
        assert_eq!(sanitize_github_query("\u{130}repo:"), "\u{130}repo:");
        assert_eq!(
            sanitize_github_query("\u{130} repo:owner/name tail"),
            "\u{130} tail"
        );
        assert_eq!(
            sanitize_github_query("\u{130}\u{130}\u{130} user:someone"),
            "\u{130}\u{130}\u{130}"
        );
    }

    use crate::test_support::{env_guard, set_env_var};
    use reqwest::header::AUTHORIZATION;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_build_raw_url_default() {
        let _g = env_guard();
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", None);
        let url = build_raw_url("owner/repo", "skills/test/SKILL.md").unwrap();
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/owner/repo/HEAD/skills/test/SKILL.md"
        );
    }

    #[test]
    #[serial]
    fn test_build_raw_url_with_custom_base() {
        let _g = env_guard();
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", Some("http://localhost:8080"));
        let url = build_raw_url("owner/repo", "skills/test/SKILL.md").unwrap();
        assert_eq!(
            url,
            "http://localhost:8080/owner/repo/HEAD/skills/test/SKILL.md"
        );
    }

    /// IN-23: a space or `#` in the repository path must be percent-encoded,
    /// or the URL names a different resource (`#` starts a fragment).
    #[test]
    #[serial]
    fn build_raw_url_percent_encodes_path_segments() {
        let _g = env_guard();
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", None);
        assert_eq!(
            build_raw_url("owner/repo", "skills/my skill/SKILL.md").unwrap(),
            "https://raw.githubusercontent.com/owner/repo/HEAD/skills/my%20skill/SKILL.md"
        );
        assert_eq!(
            build_raw_url("owner/repo", "skills/c#/SKILL.md").unwrap(),
            "https://raw.githubusercontent.com/owner/repo/HEAD/skills/c%23/SKILL.md"
        );
    }

    /// IN-22: operators count only at token starts. `in:` inside `plugin:`
    /// and `built-in:` is plain text to GitHub.
    #[test]
    fn sanitize_github_query_matches_operators_only_at_token_start() {
        assert_eq!(sanitize_github_query("plugin:foo bar"), "plugin:foo bar");
        assert_eq!(sanitize_github_query("built-in: tools"), "built-in: tools");
        // Real operators, including a negated one, are still stripped.
        assert_eq!(sanitize_github_query("foo -repo:x bar"), "foo - bar");
        assert_eq!(sanitize_github_query("(repo:x) foo"), "( foo");
        assert_eq!(sanitize_github_query("foo in:path"), "foo");
    }

    /// IN-4: the base-URL overrides exist for the wiremock tests. Whatever a
    /// release build does with them, they must never name a non-https host
    /// by default.
    #[test]
    #[serial]
    fn default_bases_are_https_github_hosts() {
        let _g = env_guard();
        let _a = set_env_var("GITHUB_API_BASE_URL", None);
        let _r = set_env_var("GITHUB_RAW_BASE_URL", None);
        assert_eq!(github_api_base(), "https://api.github.com");
        assert_eq!(raw_content_base(), "https://raw.githubusercontent.com");
    }

    #[test]
    #[serial]
    fn test_github_api_base_default() {
        let _g = env_guard();
        let _api_guard = set_env_var("GITHUB_API_BASE_URL", None);
        assert_eq!(github_api_base(), "https://api.github.com");
    }

    #[test]
    #[serial]
    fn test_github_api_base_custom() {
        let _g = env_guard();
        let _api_guard = set_env_var("GITHUB_API_BASE_URL", Some("http://localhost:9090"));
        assert_eq!(github_api_base(), "http://localhost:9090");
    }

    #[test]
    #[serial]
    fn test_github_auth_header_set_when_token_present() {
        let _g = env_guard();
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
        let _g = env_guard();
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
        let _g = env_guard();
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
        let _g = env_guard();
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
        let _g = env_guard();
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

        /// Property (IN-78): characters whose lowercase form has a different
        /// byte length, placed right before an operator, never panic the
        /// sanitizer and never let the operator through. `\\PC*` almost
        /// never generates this combination.
        #[test]
        fn sanitize_handles_case_folding_chars_next_to_operators(
            folding in prop::collection::vec(
                prop::sample::select(vec!['\u{130}', '\u{1E9E}', '\u{212A}', '\u{2126}', '\u{23A}', 'ß', 'Σ']),
                0..6,
            ),
            operator in prop::sample::select(vec!["repo:", "user:", "in:", "path:", "REPO:"]),
            value in "[a-z0-9/]{1,10}",
            spaced in any::<bool>(),
        ) {
            let prefix: String = folding.iter().collect();
            let sep = if spaced { " " } else { "" };
            let input = format!("{prefix}{sep}{operator}{value} tail");
            let result = sanitize_github_query(&input);
            prop_assert!(result.ends_with("tail"));
            if spaced || prefix.is_empty() {
                prop_assert!(
                    !result.to_ascii_lowercase().contains(&operator.to_ascii_lowercase()),
                    "operator survived: {result:?} from {input:?}"
                );
            }
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
    use crate::test_support::set_env_var;
    use serde_json::json;
    use serial_test::serial;
    use wiremock::matchers::{method, path, query_param_contains};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", Some(&server.uri()));
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

    /// IN-3: a URL off the raw content host is refused before any request,
    /// so the bearer token cannot leak to it.
    #[tokio::test]
    #[serial]
    async fn fetch_skill_content_refuses_foreign_host_without_sending_token() {
        let server = MockServer::start().await;
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", None);
        let _token_guard = set_env_var("GITHUB_TOKEN", Some("secret-token"));

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("---\n"))
            .expect(0)
            .mount(&server)
            .await;

        let url = format!("{}/owner/repo/HEAD/SKILL.md", server.uri());
        let err = fetch_skill_content(&url).await.unwrap_err().to_string();
        assert!(err.contains("refusing"), "unexpected error: {err}");

        // Plain http on the right host name is refused too.
        let err = fetch_skill_content("http://raw.githubusercontent.com/o/r/HEAD/SKILL.md")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("refusing"), "unexpected error: {err}");
        server.verify().await;
    }

    /// IN-5: an oversized body is refused instead of buffered whole.
    #[tokio::test]
    #[serial]
    async fn fetch_skill_content_refuses_oversized_body() {
        let server = MockServer::start().await;
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", Some(&server.uri()));
        let _token_guard = set_env_var("GITHUB_TOKEN", None);

        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("x".repeat(MAX_BODY_BYTES + 1)),
            )
            .mount(&server)
            .await;

        let url = format!("{}/owner/repo/HEAD/SKILL.md", server.uri());
        let err = fetch_skill_content(&url).await.unwrap_err().to_string();
        assert!(err.contains("limit"), "unexpected error: {err}");
    }

    /// IN-5: a stalled server fails the request once the timeout elapses.
    #[tokio::test]
    #[serial]
    async fn client_times_out_on_stalled_server() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(10)))
            .mount(&server)
            .await;

        let client = build_client(std::time::Duration::from_millis(200)).unwrap();
        let started = std::time::Instant::now();
        let result = client.get(server.uri()).send().await;
        assert!(result.is_err(), "a 10s delay must hit a 200ms timeout");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[tokio::test]
    #[serial]
    async fn test_fetch_skill_content_not_found() {
        let server = MockServer::start().await;
        let _raw_guard = set_env_var("GITHUB_RAW_BASE_URL", Some(&server.uri()));
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

        result.expect("search with sanitized injected operators should succeed");
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
        result.expect("search with auth header should succeed");
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
