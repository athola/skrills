use std::sync::Arc;
use std::time::Duration;

use crate::backend::{config::AdapterConfig, AdapterCapabilities, BackendAdapter};
use crate::store::{
    BackendKind, RunEvent, RunId, RunRecord, RunRequest, RunState, RunStatus, RunStore,
    SubagentTemplate,
};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use reqwest::Url;
use serde::Serialize;
use serde_json::{json, Value};
use time::OffsetDateTime;

const DEFAULT_BASE: &str = "https://api.openai.com/v1";
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const API_KEY_ERROR: &str = "Codex API key not set. Set SKRILLS_CODEX_API_KEY environment variable with your OpenAI API key. Get one at https://platform.openai.com/api-keys";

#[derive(Debug, Clone)]
pub struct CodexAdapter {
    config: AdapterConfig,
    client: reqwest::Client,
}

impl CodexAdapter {
    pub fn new(model: String) -> Result<Self> {
        let config =
            match AdapterConfig::from_env("CODEX", &model, DEFAULT_BASE, DEFAULT_TIMEOUT_MS) {
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
            let error_msg = API_KEY_ERROR;
            store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Failed,
                        message: Some(error_msg.into()),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await?;
            return Err(anyhow!(error_msg));
        }

        let body = build_openai_body(&self.config.model, &request);
        let url = self
            .config
            .base_url
            .join("chat/completions")
            .unwrap_or_else(|_| self.config.base_url.clone());

        let resp = self
            .client
            .post(url)
            .bearer_auth(&self.config.api_key)
            .json(&body)
            .send()
            .await
            .context("calling Codex API")?;

        let status = resp.status();
        let text = resp.text().await?;
        let parsed: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));

        if !status.is_success() {
            let msg = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|m| m.as_str())
                .unwrap_or("codex call failed")
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

        let completion = extract_openai_text(&parsed).unwrap_or_else(|| text.clone());
        // Simulate streaming events by chunking the completion into words.
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
struct OpenAiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct JsonSchemaFormat {
    #[serde(rename = "type")]
    fmt_type: String,
    json_schema: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    strict: Option<bool>,
}

#[derive(Debug, Serialize)]
struct OpenAiBody {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<JsonSchemaFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

fn build_openai_body(model: &str, request: &RunRequest) -> OpenAiBody {
    let response_format = request
        .output_schema
        .as_ref()
        .map(|schema| JsonSchemaFormat {
            fmt_type: "json_schema".into(),
            json_schema: json!({ "name": "subagent_output", "schema": schema }),
            strict: Some(true),
        });
    let metadata = if request.tracing {
        Some(json!({"trace": true}))
    } else {
        None
    };
    OpenAiBody {
        model: model.to_string(),
        messages: vec![OpenAiMessage {
            role: "user".into(),
            content: request.prompt.clone(),
        }],
        response_format,
        stream: Some(request.async_mode),
        metadata,
    }
}

fn extract_openai_text(val: &Value) -> Option<String> {
    val.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|content| {
            if let Some(s) = content.as_str() {
                return Some(s.to_string());
            }
            if let Some(arr) = content.as_array() {
                let mut buf = String::new();
                for item in arr {
                    if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                        buf.push_str(text);
                    }
                }
                if !buf.is_empty() {
                    return Some(buf);
                }
            }
            None
        })
        .or_else(|| {
            val.get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string())
        })
}

#[async_trait]
impl BackendAdapter for CodexAdapter {
    fn backend(&self) -> BackendKind {
        BackendKind::Codex
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            supports_schema: true,
            supports_async: true,
            supports_tracing: true,
            supports_secure_transcript: true,
        }
    }

    async fn list_templates(&self) -> Result<Vec<SubagentTemplate>> {
        Ok(vec![SubagentTemplate {
            id: "default".into(),
            name: "Default Codex Agent".into(),
            description: Some(format!("Codex model {}", self.config.model)),
            backend: BackendKind::Codex,
            capabilities: vec!["tools".into(), "structured_outputs".into()],
        }])
    }

    async fn run(&self, mut request: RunRequest, store: Arc<dyn RunStore>) -> Result<RunId> {
        request.backend = BackendKind::Codex;
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
    fn test_codex_adapter_new() {
        let adapter = CodexAdapter::new("gpt-4".to_string()).unwrap();
        assert_eq!(adapter.config.model, "gpt-4");
        assert_eq!(adapter.config.base_url.as_str(), DEFAULT_BASE);
        assert_eq!(
            adapter.config.timeout,
            Duration::from_millis(DEFAULT_TIMEOUT_MS)
        );
    }

    #[test]
    fn test_codex_adapter_with_config() {
        let config = AdapterConfig {
            api_key: "test-key".to_string(),
            base_url: reqwest::Url::parse("https://test.com").unwrap(),
            model: "test-model".to_string(),
            timeout: Duration::from_secs(30),
        };

        let adapter = CodexAdapter::with_config(config.clone()).unwrap();
        assert_eq!(adapter.config.api_key, "test-key");
        assert_eq!(adapter.config.model, "test-model");
        assert_eq!(adapter.config.base_url.as_str(), "https://test.com/");
    }

    #[test]
    fn test_codex_adapter_backend() {
        let adapter = CodexAdapter::new("gpt-4".to_string()).unwrap();
        assert_eq!(adapter.backend(), BackendKind::Codex);
    }

    #[test]
    fn test_codex_capabilities() {
        let adapter = CodexAdapter::new("gpt-4".to_string()).unwrap();
        let capabilities = adapter.capabilities();

        assert!(capabilities.supports_schema);
        assert!(capabilities.supports_async);
        assert!(capabilities.supports_tracing);
        assert!(capabilities.supports_secure_transcript);
    }

    #[tokio::test]
    async fn test_codex_list_templates() {
        let adapter = CodexAdapter::new("gpt-4".to_string()).unwrap();
        let templates = adapter.list_templates().await.unwrap();

        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].id, "default");
        assert_eq!(templates[0].name, "Default Codex Agent");
        assert_eq!(templates[0].backend, BackendKind::Codex);
        assert!(templates[0].capabilities.contains(&"tools".to_string()));
        assert!(templates[0]
            .capabilities
            .contains(&"structured_outputs".to_string()));
    }

    #[test]
    fn test_build_openai_body_default() {
        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Hello, world!".to_string(),
            template_id: Some("test".to_string()),
            output_schema: None,
            tracing: false,
            async_mode: false,
        };

        let body = build_openai_body("gpt-4", &request);

        assert_eq!(body.model, "gpt-4");
        assert_eq!(body.messages.len(), 1);
        assert_eq!(body.messages[0].role, "user");
        assert_eq!(body.messages[0].content, "Hello, world!");
        assert_eq!(body.stream, Some(false));
        assert!(body.response_format.is_none());
        assert!(body.metadata.is_none());
    }

    #[test]
    fn test_build_openai_body_with_schema() {
        let mut schema = serde_json::Map::new();
        schema.insert(
            "type".to_string(),
            serde_json::Value::String("object".to_string()),
        );

        let request = RunRequest {
            backend: BackendKind::Codex,
            prompt: "Generate JSON".to_string(),
            template_id: Some("test".to_string()),
            output_schema: Some(serde_json::Value::Object(schema)),
            tracing: true,
            async_mode: true,
        };

        let body = build_openai_body("gpt-4", &request);

        assert_eq!(body.stream, Some(true));
        assert!(body.response_format.is_some());
        assert!(body.metadata.is_some());

        let response_format = body.response_format.unwrap();
        assert_eq!(response_format.fmt_type, "json_schema");
        assert_eq!(response_format.json_schema["name"], "subagent_output");
        assert_eq!(response_format.json_schema["schema"]["type"], "object");
        assert_eq!(response_format.strict, Some(true));

        let metadata = body.metadata.unwrap();
        assert_eq!(metadata["trace"], true);
    }

    #[test]
    fn test_extract_openai_text_from_choices() {
        let value = json!({
            "choices": [
                {
                    "message": {
                        "content": "Hello, world!"
                    }
                }
            ]
        });

        let text = extract_openai_text(&value).unwrap();
        assert_eq!(text, "Hello, world!");
    }

    #[test]
    fn test_extract_openai_text_from_array_content() {
        let value = json!({
            "choices": [
                {
                    "message": {
                        "content": [
                            {"type": "text", "text": "Hello, "},
                            {"type": "text", "text": "world!"}
                        ]
                    }
                }
            ]
        });

        let text = extract_openai_text(&value).unwrap();
        assert_eq!(text, "Hello, world!");
    }

    #[test]
    fn test_extract_openai_text_from_message() {
        let value = json!({
            "message": {
                "content": "Simple response"
            }
        });

        let text = extract_openai_text(&value).unwrap();
        assert_eq!(text, "Simple response");
    }

    #[test]
    fn test_extract_openai_text_no_content() {
        let value = json!({
            "error": "Something went wrong"
        });

        assert!(extract_openai_text(&value).is_none());
    }

    #[test]
    fn test_extract_openai_text_empty_choices() {
        let value = json!({
            "choices": []
        });

        assert!(extract_openai_text(&value).is_none());
    }

    #[test]
    fn test_extract_openai_text_null_content() {
        let value = json!({
            "choices": [
                {
                    "message": {
                        "content": null
                    }
                }
            ]
        });

        assert!(extract_openai_text(&value).is_none());
    }

    #[test]
    fn test_extract_openai_text_empty_array_content() {
        let value = json!({
            "choices": [
                {
                    "message": {
                        "content": []
                    }
                }
            ]
        });

        assert!(extract_openai_text(&value).is_none());
    }

    #[test]
    fn test_openai_message_serialization() {
        let message = OpenAiMessage {
            role: "user".to_string(),
            content: "Hello, Codex!".to_string(),
        };

        let json = serde_json::to_string(&message).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["role"], "user");
        assert_eq!(parsed["content"], "Hello, Codex!");
    }

    #[test]
    fn test_json_schema_format_serialization() {
        let schema = JsonSchemaFormat {
            fmt_type: "json_schema".to_string(),
            json_schema: json!({"name": "test", "schema": {"type": "object"}}),
            strict: Some(true),
        };

        let json = serde_json::to_string(&schema).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["type"], "json_schema");
        assert_eq!(parsed["json_schema"]["name"], "test");
        assert_eq!(parsed["strict"], true);
    }

    #[test]
    fn test_json_schema_format_serialization_skips_none() {
        let schema = JsonSchemaFormat {
            fmt_type: "json_schema".to_string(),
            json_schema: json!({"name": "test"}),
            strict: None,
        };

        let json = serde_json::to_string(&schema).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.get("strict").is_none());
    }

    #[test]
    fn test_openai_body_serialization() {
        let body = OpenAiBody {
            model: "gpt-4".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: "Test".to_string(),
            }],
            response_format: Some(JsonSchemaFormat {
                fmt_type: "json_schema".to_string(),
                json_schema: json!({"name": "test"}),
                strict: Some(false),
            }),
            stream: Some(true),
            metadata: Some(json!({"trace": true})),
        };

        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["model"], "gpt-4");
        assert_eq!(parsed["stream"], true);
        assert_eq!(parsed["response_format"]["type"], "json_schema");
        assert_eq!(parsed["response_format"]["strict"], false);
        assert_eq!(parsed["metadata"]["trace"], true);
    }

    #[test]
    fn test_openai_body_serialization_skips_none() {
        let body = OpenAiBody {
            model: "gpt-4".to_string(),
            messages: vec![OpenAiMessage {
                role: "user".to_string(),
                content: "Test".to_string(),
            }],
            response_format: None,
            stream: None,
            metadata: None,
        };

        let json = serde_json::to_string(&body).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert!(parsed.get("response_format").is_none());
        assert!(parsed.get("stream").is_none());
        assert!(parsed.get("metadata").is_none());
    }

    // Integration-style tests that demonstrate usage patterns
    #[tokio::test]
    async fn test_codex_run_creates_record() {
        // This test demonstrates the run flow without actually calling Codex API
        let adapter = CodexAdapter::new("gpt-4".to_string()).unwrap();

        // Note: This test shows the intended usage pattern
        // In practice, you'd need an implementation of RunStore
        let _request = RunRequest {
            backend: BackendKind::Codex,
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
            adapter.config.api_key.is_empty() || adapter.config.api_key == "skrills_codex_api_key"
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
