use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Url;
use serde::Serialize;
use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::backend::{config::AdapterConfig, AdapterCapabilities, BackendAdapter};
use crate::store::{
    BackendKind, RunEvent, RunId, RunRecord, RunRequest, RunState, RunStatus, RunStore,
    SubagentTemplate,
};

const DEFAULT_BASE: &str = "https://api.anthropic.com/v1/";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone)]
pub struct ClaudeAdapter {
    config: AdapterConfig,
    client: reqwest::Client,
}

impl ClaudeAdapter {
    pub fn new(model: String) -> Result<Self> {
        let config = AdapterConfig::from_env("CLAUDE", &model, DEFAULT_BASE, DEFAULT_TIMEOUT_MS)
            .unwrap_or_else(|_| AdapterConfig {
                api_key: String::new(),
                base_url: Url::parse(DEFAULT_BASE).expect("valid default base"),
                model,
                timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            });
        Self::with_config(config)
    }

    pub fn with_config(config: AdapterConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { config, client })
    }

    async fn execute_run(
        &self,
        run_id: RunId,
        request: RunRequest,
        store: Arc<dyn RunStore>,
    ) -> Result<()> {
        store
            .append_event(
                run_id,
                RunEvent {
                    ts: OffsetDateTime::now_utc(),
                    kind: "start".into(),
                    data: None,
                },
            )
            .await?;

        if self.config.api_key.is_empty() {
            store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Failed,
                        message: Some("missing SKRILLS_CLAUDE_API_KEY".into()),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await?;
            return Err(anyhow!("missing SKRILLS_CLAUDE_API_KEY"));
        }

        let url = self
            .config
            .base_url
            .join("messages")
            .unwrap_or_else(|_| self.config.base_url.clone());
        let body = build_anthropic_body(&self.config.model, &request);

        let resp = self
            .client
            .post(url)
            .header("x-api-key", &self.config.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .context("calling Claude API")?;

        let status = resp.status();
        let text = resp.text().await?;
        let parsed: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));

        if !status.is_success() {
            let msg = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("claude call failed")
                .to_string();
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: "error".into(),
                        data: Some(parsed.clone()),
                    },
                )
                .await?;
            store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Failed,
                        message: Some(msg.clone()),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await?;
            return Err(anyhow!(msg));
        }

        let completion = extract_anthropic_text(&parsed).unwrap_or_else(|| text.clone());
        for token in completion.split_whitespace() {
            store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: "stream".into(),
                        data: Some(json!({ "token": token })),
                    },
                )
                .await?;
        }
        store
            .append_event(
                run_id,
                RunEvent {
                    ts: OffsetDateTime::now_utc(),
                    kind: "completion".into(),
                    data: Some(json!({ "text": completion })),
                },
            )
            .await?;

        store
            .update_status(
                run_id,
                RunStatus {
                    state: RunState::Succeeded,
                    message: Some("completed".into()),
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await?;
        Ok(())
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
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<Value>,
}

fn build_anthropic_body(model: &str, request: &RunRequest) -> AnthropicBody {
    let response_format = request.output_schema.as_ref().map(|schema| {
        json!({
            "type": "json_schema",
            "json_schema": {
                "name": "subagent_output",
                "schema": schema
            }
        })
    });
    let metadata = if request.tracing {
        Some(json!({"trace": true}))
    } else {
        None
    };
    AnthropicBody {
        model: model.to_string(),
        messages: vec![AnthropicMessage {
            role: "user".into(),
            content: request.prompt.clone(),
        }],
        max_tokens: 1024,
        stream: Some(request.async_mode),
        metadata,
        response_format,
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
        let run_id = store.create_run(request.clone()).await?;
        store
            .update_status(
                run_id,
                RunStatus {
                    state: RunState::Running,
                    message: Some("dispatched".into()),
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await?;

        let cloned = self.clone();
        tokio::spawn(async move {
            if let Err(err) = cloned.execute_run(run_id, request, store.clone()).await {
                let _ = store
                    .append_event(
                        run_id,
                        RunEvent {
                            ts: OffsetDateTime::now_utc(),
                            kind: "error".into(),
                            data: Some(json!({"message": err.to_string()})),
                        },
                    )
                    .await;
                let _ = store
                    .update_status(
                        run_id,
                        RunStatus {
                            state: RunState::Failed,
                            message: Some(err.to_string()),
                            updated_at: OffsetDateTime::now_utc(),
                        },
                    )
                    .await;
            }
        });

        Ok(run_id)
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

        let body = build_anthropic_body("claude-3-haiku-20240307", &request);

        assert_eq!(body.model, "claude-3-haiku-20240307");
        assert_eq!(body.messages.len(), 1);
        assert_eq!(body.messages[0].role, "user");
        assert_eq!(body.messages[0].content, "Hello, world!");
        assert_eq!(body.max_tokens, 1024);
        assert_eq!(body.stream, Some(false));
        assert!(body.metadata.is_none());
        assert!(body.response_format.is_none());
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

        let body = build_anthropic_body("claude-3-haiku-20240307", &request);

        assert_eq!(body.stream, Some(true));
        assert!(body.metadata.is_some());
        assert!(body.response_format.is_some());

        let response_format = body.response_format.unwrap();
        assert_eq!(response_format["type"], "json_schema");
        assert_eq!(response_format["json_schema"]["name"], "subagent_output");
        assert_eq!(response_format["json_schema"]["schema"]["type"], "object");

        let metadata = body.metadata.unwrap();
        assert_eq!(metadata["trace"], true);
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
            metadata: Some(json!({"trace": true})),
            response_format: None,
        };

        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["model"], "claude-3-haiku-20240307");
        assert_eq!(parsed["max_tokens"], 100);
        assert_eq!(parsed["stream"], true);
        assert_eq!(parsed["metadata"]["trace"], true);
        assert!(parsed.get("response_format").is_none());
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
            metadata: None,
            response_format: None,
        };

        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.get("stream").is_none());
        assert!(parsed.get("metadata").is_none());
        assert!(parsed.get("response_format").is_none());
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
}
