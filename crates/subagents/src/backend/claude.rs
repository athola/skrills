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
    pub fn new(model: String) -> Self {
        let config = AdapterConfig::from_env("CLAUDE", &model, DEFAULT_BASE, DEFAULT_TIMEOUT_MS)
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
