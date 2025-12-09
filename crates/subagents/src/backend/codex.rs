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

#[derive(Debug, Clone)]
pub struct CodexAdapter {
    config: AdapterConfig,
    client: reqwest::Client,
}

impl CodexAdapter {
    pub fn new(model: String) -> Self {
        let config = AdapterConfig::from_env("CODEX", &model, DEFAULT_BASE, DEFAULT_TIMEOUT_MS)
            .unwrap_or_else(|_| AdapterConfig {
                api_key: String::new(),
                base_url: Url::parse(DEFAULT_BASE).expect("valid default base"),
                model,
                timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            });
        Self::with_config(config)
    }

    pub fn with_config(config: AdapterConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .expect("failed to build reqwest client");
        Self { config, client }
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
                        message: Some("missing SKRILLS_CODEX_API_KEY".into()),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await?;
            return Err(anyhow!("missing SKRILLS_CODEX_API_KEY"));
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

    async fn get_status(
        &self,
        run_id: RunId,
        store: Arc<dyn RunStore>,
    ) -> Result<Option<RunStatus>> {
        store.get_status(run_id).await
    }

    async fn stop(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<bool> {
        store.stop(run_id).await
    }

    async fn history(&self, limit: usize, store: Arc<dyn RunStore>) -> Result<Vec<RunStatus>> {
        let runs: Vec<RunRecord> = store.history(limit).await?;
        Ok(runs.into_iter().map(|r| r.status).collect())
    }
}
