use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Url;
use serde::Serialize;
use serde_json::{json, Value};

use crate::backend::{
    config::{endpoint, AdapterConfig},
    run_http_adapter, spawn_run, AdapterCapabilities, BackendAdapter, HttpProvider,
};
use crate::store::{
    BackendKind, RunId, RunRecord, RunRequest, RunStatus, RunStore, SubagentTemplate,
};

const DEFAULT_BASE: &str = "https://api.anthropic.com/v1/";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const ANTHROPIC_VERSION: &str = "2023-06-01";
/// Default output cap. A non-streaming request with a much larger cap risks the
/// HTTP timeout; `SKRILLS_CLAUDE_MAX_TOKENS` overrides it.
const DEFAULT_MAX_TOKENS: u32 = 16_000;
const API_KEY_ERROR: &str = "Claude API key not set. Set SKRILLS_CLAUDE_API_KEY environment variable with your Anthropic API key. Get one at https://console.anthropic.com/settings/keys";

#[derive(Debug, Clone)]
pub struct ClaudeAdapter {
    config: AdapterConfig,
    client: reqwest::Client,
    max_tokens: u32,
}

/// Reads `SKRILLS_CLAUDE_MAX_TOKENS`, warning on a value that is not a
/// positive whole number.
fn max_tokens_from_env() -> u32 {
    match std::env::var("SKRILLS_CLAUDE_MAX_TOKENS") {
        Err(_) => DEFAULT_MAX_TOKENS,
        Ok(raw) => match raw.trim().parse::<u32>() {
            Ok(n) if n > 0 => n,
            _ => {
                tracing::warn!(
                    value = %raw,
                    default = DEFAULT_MAX_TOKENS,
                    "invalid SKRILLS_CLAUDE_MAX_TOKENS; using the default"
                );
                DEFAULT_MAX_TOKENS
            }
        },
    }
}

impl ClaudeAdapter {
    pub fn new(model: String) -> Result<Self> {
        let config =
            match AdapterConfig::from_env("CLAUDE", &model, DEFAULT_BASE, DEFAULT_TIMEOUT_MS) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(error = %e, "adapter config unavailable; API calls will fail");
                    AdapterConfig {
                        api_key: String::new(),
                        base_url: Url::parse(DEFAULT_BASE)
                            .expect("DEFAULT_BASE constant must be a valid URL"),
                        model,
                        timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
                    }
                }
            };
        Self::with_config(config)
    }

    pub fn with_config(config: AdapterConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self {
            config,
            client,
            max_tokens: max_tokens_from_env(),
        })
    }

    async fn execute_run(
        &self,
        run_id: RunId,
        request: RunRequest,
        store: Arc<dyn RunStore>,
    ) -> Result<()> {
        let url = endpoint(&self.config.base_url, "messages")?;
        let body = build_anthropic_body(&self.config.model, self.max_tokens, &request);
        let api_key = self.config.api_key.clone();

        run_http_adapter(run_id, &store, &self.config.api_key, &PROVIDER, || {
            self.client
                .post(url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(&body)
        })
        .await
    }
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct AnthropicBody {
    model: String,
    messages: Vec<AnthropicMessage>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    /// Structured output: `{"format": {"type": "json_schema", "schema": ...}}`.
    /// The Messages API has no OpenAI-style `response_format`, and its
    /// `metadata` accepts only `user_id`, so neither is sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<Value>,
}

fn build_anthropic_body(model: &str, max_tokens: u32, request: &RunRequest) -> AnthropicBody {
    let output_config = request.output_schema.as_ref().map(|schema| {
        json!({
            "format": {
                "type": "json_schema",
                "schema": schema
            }
        })
    });
    AnthropicBody {
        model: model.to_string(),
        messages: vec![AnthropicMessage {
            role: "user".into(),
            content: request.prompt.clone(),
        }],
        max_tokens,
        stream: None, // streaming not currently supported; do not conflate with async_mode
        output_config,
    }
}

fn extract_anthropic_text(val: &Value) -> Option<String> {
    val.get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| {
            let mut buf = String::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    buf.push_str(text);
                }
            }
            if buf.is_empty() {
                None
            } else {
                Some(buf)
            }
        })
        .or_else(|| {
            val.get("content")
                .and_then(|c| c.as_str())
                .map(|s| s.to_string())
        })
}

const PROVIDER: HttpProvider = HttpProvider {
    label: "Claude",
    api_key_error: API_KEY_ERROR,
    truncated_reason: "max_tokens",
    extract_text: extract_anthropic_text,
    extract_stop_reason: anthropic_stop_reason,
};

fn anthropic_stop_reason(val: &Value) -> Option<String> {
    val.get("stop_reason")
        .and_then(|r| r.as_str())
        .map(str::to_string)
}

#[async_trait]
impl BackendAdapter for ClaudeAdapter {
    fn backend(&self) -> BackendKind {
        BackendKind::Claude
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            supports_schema: true,
            supports_async: true,
            supports_tracing: false,
            supports_secure_transcript: false,
        }
    }

    async fn list_templates(&self) -> Result<Vec<SubagentTemplate>> {
        Ok(vec![SubagentTemplate {
            id: "default".into(),
            name: "Claude Code Subagent".into(),
            description: Some(format!("Claude model {}", self.config.model)),
            backend: BackendKind::Claude,
            capabilities: vec!["tools".into(), "structured_outputs".into()],
        }])
    }

    async fn run(&self, mut request: RunRequest, store: Arc<dyn RunStore>) -> Result<RunId> {
        request.backend = BackendKind::Claude;
        let adapter = self.clone();
        spawn_run(
            request,
            store,
            "dispatched",
            "Claude",
            move |run_id, request, store| async move {
                adapter.execute_run(run_id, request, store).await
            },
        )
        .await
    }

    async fn status(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<Option<RunStatus>> {
        store.status(run_id).await
    }

    async fn stop(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<bool> {
        store.stop(run_id).await
    }

    async fn history(&self, limit: usize, store: Arc<dyn RunStore>) -> Result<Vec<RunStatus>> {
        let runs: Vec<RunRecord> = store.history(limit).await?;
        Ok(runs.into_iter().map(|r| r.status).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_claude_adapter_new() {
        let adapter = ClaudeAdapter::new("claude-3-haiku-20240307".to_string()).unwrap();
        assert_eq!(adapter.config.model, "claude-3-haiku-20240307");
        assert_eq!(adapter.config.base_url.as_str(), DEFAULT_BASE);
        assert_eq!(
            adapter.config.timeout,
            Duration::from_millis(DEFAULT_TIMEOUT_MS)
        );
    }

    #[test]
    fn test_claude_adapter_with_config() {
        let config = AdapterConfig {
            api_key: "test-key".to_string(),
            base_url: reqwest::Url::parse("https://test.com").unwrap(),
            model: "test-model".to_string(),
            timeout: Duration::from_secs(30),
        };

        let adapter = ClaudeAdapter::with_config(config.clone()).unwrap();
        assert_eq!(adapter.config.api_key, "test-key");
        assert_eq!(adapter.config.model, "test-model");
        assert_eq!(adapter.config.base_url.as_str(), "https://test.com/");
    }

    #[test]
    fn test_claude_adapter_backend() {
        let adapter = ClaudeAdapter::new("claude-3-haiku-20240307".to_string()).unwrap();
        assert_eq!(adapter.backend(), BackendKind::Claude);
    }

    #[test]
    fn test_claude_capabilities() {
        let adapter = ClaudeAdapter::new("claude-3-haiku-20240307".to_string()).unwrap();
        let capabilities = adapter.capabilities();

        assert!(capabilities.supports_schema);
        assert!(capabilities.supports_async);
        assert!(!capabilities.supports_tracing);
        assert!(!capabilities.supports_secure_transcript);
    }

    #[tokio::test]
    async fn test_claude_list_templates() {
        let adapter = ClaudeAdapter::new("claude-3-haiku-20240307".to_string()).unwrap();
        let templates = adapter.list_templates().await.unwrap();

        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].id, "default");
        assert_eq!(templates[0].name, "Claude Code Subagent");
        assert_eq!(templates[0].backend, BackendKind::Claude);
        assert!(templates[0].capabilities.contains(&"tools".to_string()));
        assert!(templates[0]
            .capabilities
            .contains(&"structured_outputs".to_string()));
    }

    #[test]
    fn test_build_anthropic_body_default() {
        let request = RunRequest {
            backend: BackendKind::Claude,
            prompt: "Hello, world!".to_string(),
            template_id: Some("test".to_string()),
            output_schema: None,
            tracing: false,
            async_mode: false,
        };

        let body = build_anthropic_body("claude-3-haiku-20240307", DEFAULT_MAX_TOKENS, &request);

        assert_eq!(body.model, "claude-3-haiku-20240307");
        assert_eq!(body.messages.len(), 1);
        assert_eq!(body.messages[0].role, "user");
        assert_eq!(body.messages[0].content, "Hello, world!");
        assert_eq!(body.max_tokens, DEFAULT_MAX_TOKENS);
        assert_eq!(body.stream, None); // streaming not supported
        assert!(body.output_config.is_none());
    }

    #[test]
    fn test_build_anthropic_body_with_schema() {
        let mut schema = serde_json::Map::new();
        schema.insert(
            "type".to_string(),
            serde_json::Value::String("object".to_string()),
        );

        let request = RunRequest {
            backend: BackendKind::Claude,
            prompt: "Generate JSON".to_string(),
            template_id: Some("test".to_string()),
            output_schema: Some(serde_json::Value::Object(schema)),
            tracing: true,
            async_mode: true,
        };

        let body = build_anthropic_body("claude-3-haiku-20240307", 512, &request);

        assert_eq!(body.stream, None); // streaming not supported
        assert_eq!(body.max_tokens, 512);

        // RT-20: the Messages API takes `output_config.format`; an OpenAI-style
        // `response_format` or a `metadata.trace` key is rejected with a 400.
        let parsed = serde_json::to_value(&body).unwrap();
        assert_eq!(parsed["output_config"]["format"]["type"], "json_schema");
        assert_eq!(
            parsed["output_config"]["format"]["schema"]["type"],
            "object"
        );
        assert!(parsed.get("response_format").is_none());
        assert!(parsed.get("metadata").is_none());
    }

    #[test]
    fn test_extract_anthropic_text_from_content_array() {
        let value = json!({
            "content": [
                {"type": "text", "text": "Hello, "},
                {"type": "text", "text": "world!"}
            ]
        });

        let text = extract_anthropic_text(&value).unwrap();
        assert_eq!(text, "Hello, world!");
    }

    #[test]
    fn test_extract_anthropic_text_from_string_content() {
        let value = json!({
            "content": "Simple text response"
        });

        let text = extract_anthropic_text(&value).unwrap();
        assert_eq!(text, "Simple text response");
    }

    #[test]
    fn test_extract_anthropic_text_no_content() {
        let value = json!({
            "error": "Something went wrong"
        });

        assert!(extract_anthropic_text(&value).is_none());
    }

    #[test]
    fn test_extract_anthropic_text_empty_content_array() {
        let value = json!({
            "content": []
        });

        assert!(extract_anthropic_text(&value).is_none());
    }

    #[test]
    fn test_extract_anthropic_text_content_array_without_text() {
        let value = json!({
            "content": [
                {"type": "image", "source": "data:image/png;base64,..."}
            ]
        });

        assert!(extract_anthropic_text(&value).is_none());
    }

    #[test]
    fn test_anthropic_message_serialization() {
        let message = AnthropicMessage {
            role: "user".to_string(),
            content: "Hello, Claude!".to_string(),
        };

        let json = serde_json::to_string(&message).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["role"], "user");
        assert_eq!(parsed["content"], "Hello, Claude!");
    }

    #[test]
    fn test_anthropic_body_serialization() {
        let body = AnthropicBody {
            model: "claude-3-haiku-20240307".to_string(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: "Test".to_string(),
            }],
            max_tokens: 100,
            stream: Some(true),
            output_config: None,
        };

        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["model"], "claude-3-haiku-20240307");
        assert_eq!(parsed["max_tokens"], 100);
        assert_eq!(parsed["stream"], true);
        assert!(parsed.get("output_config").is_none());
    }

    #[test]
    fn test_anthropic_body_serialization_skips_none() {
        let body = AnthropicBody {
            model: "claude-3-haiku-20240307".to_string(),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: "Test".to_string(),
            }],
            max_tokens: 100,
            stream: None,
            output_config: None,
        };

        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.get("stream").is_none());
        assert!(parsed.get("output_config").is_none());
    }

    // Integration-style tests that demonstrate usage patterns
    #[tokio::test]
    async fn test_claude_run_creates_record() {
        // This test demonstrates the run flow without actually calling Claude API
        let adapter = ClaudeAdapter::new("claude-3-haiku-20240307".to_string()).unwrap();

        // Note: This test shows the intended usage pattern
        // In practice, you'd need an implementation of RunStore
        let _request = RunRequest {
            backend: BackendKind::Claude,
            prompt: "Test prompt".to_string(),
            template_id: Some("test".to_string()),
            output_schema: None,
            tracing: false,
            async_mode: false,
        };

        // The run method would:
        // 1. Create a run record in the store
        // 2. Update status to Running
        // 3. Spawn a task to execute the run
        // 4. Return the run ID

        // Note: Actual execution requires a valid API key
        assert!(
            adapter.config.api_key.is_empty() || adapter.config.api_key == "skrills_claude_api_key"
        );
    }

    #[test]
    fn default_base_url_is_valid() {
        // Validates that DEFAULT_BASE is a well-formed URL at test time.
        // This documents the invariant and catches any changes to the const
        // that would break URL parsing.
        assert!(
            Url::parse(DEFAULT_BASE).is_ok(),
            "DEFAULT_BASE must be a valid URL: {}",
            DEFAULT_BASE
        );
    }
}
