//! Search GitHub for existing SKILL.md files.

use anyhow::Result;
use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};

const GITHUB_API_BASE: &str = "https://api.github.com";

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
    #[allow(dead_code)]
    html_url: String,
    repository: Repository,
}

#[derive(Deserialize)]
struct Repository {
    full_name: String,
    html_url: String,
    stargazers_count: u64,
    updated_at: String,
    description: Option<String>,
}

/// Search GitHub for skills matching the query.
pub async fn search_github_skills(query: &str, limit: usize) -> Result<Vec<GitHubSkillResult>> {
    // Build search query for SKILL.md files
    let search_query = format!("{} filename:SKILL.md", query);

    let client = reqwest::Client::new();

    let response = apply_github_auth(
        client
            .get(format!("{}/search/code", GITHUB_API_BASE))
            .query(&[
                ("q", search_query.as_str()),
                ("per_page", &limit.to_string()),
            ])
            .header("Accept", "application/vnd.github.v3+json")
            .header("User-Agent", "skrills-intelligence/0.4.0"),
    )
    .send()
    .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API error ({}): {}", status, body);
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
    format!(
        "https://raw.githubusercontent.com/{}/HEAD/{}",
        full_name, path
    )
}

/// Fetch the content of a skill from its raw URL.
#[allow(dead_code)]
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
#[allow(dead_code)]
pub async fn search_skills_advanced(
    keywords: &[String],
    language: Option<&str>,
    min_stars: Option<u64>,
    limit: usize,
) -> Result<Vec<GitHubSkillResult>> {
    let mut query_parts = vec!["filename:SKILL.md".to_string()];

    // Add keywords
    for keyword in keywords {
        query_parts.push(keyword.clone());
    }

    // Add language filter
    if let Some(lang) = language {
        query_parts.push(format!("language:{}", lang));
    }

    // Add star filter
    if let Some(stars) = min_stars {
        query_parts.push(format!("stars:>={}", stars));
    }

    let query = query_parts.join(" ");
    search_github_skills(&query, limit).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::AUTHORIZATION;
    use std::env;
    use std::sync::LazyLock;
    use std::sync::Mutex;

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn env_guard() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

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
    fn test_build_raw_url() {
        let url = build_raw_url("owner/repo", "skills/test/SKILL.md");
        assert_eq!(
            url,
            "https://raw.githubusercontent.com/owner/repo/HEAD/skills/test/SKILL.md"
        );
    }

    #[test]
    fn test_github_auth_header_set_when_token_present() {
        let _guard = env_guard();
        let _token_guard = set_env_var("GITHUB_TOKEN", Some("test-token"));
        let client = reqwest::Client::new();
        let request = apply_github_auth(client.get("https://api.github.com"))
            .build()
            .unwrap();
        let header = request.headers().get(AUTHORIZATION).unwrap();
        assert_eq!(header.to_str().unwrap(), "Bearer test-token");
    }

    #[test]
    fn test_github_auth_header_absent_when_no_token() {
        let _guard = env_guard();
        let _token_guard = set_env_var("GITHUB_TOKEN", None);
        let client = reqwest::Client::new();
        let request = apply_github_auth(client.get("https://api.github.com"))
            .build()
            .unwrap();
        assert!(request.headers().get(AUTHORIZATION).is_none());
    }
}
