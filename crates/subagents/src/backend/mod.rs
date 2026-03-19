pub mod claude;
pub mod cli;
pub mod codex;
pub mod config;

use std::sync::Arc;

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use reqwest::RequestBuilder;
use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::store::{
    BackendKind, RunEvent, RunId, RunRequest, RunState, RunStatus, RunStore, SubagentTemplate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterCapabilities {
    pub supports_schema: bool,
    pub supports_async: bool,
    pub supports_tracing: bool,
    pub supports_secure_transcript: bool,
}

#[async_trait]
pub trait BackendAdapter: Send + Sync {
    fn backend(&self) -> BackendKind;
    fn capabilities(&self) -> AdapterCapabilities;
    async fn list_templates(&self) -> Result<Vec<SubagentTemplate>>;
    async fn run(&self, request: RunRequest, store: Arc<dyn RunStore>) -> Result<RunId>;
    async fn status(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<Option<RunStatus>>;
    async fn stop(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<bool>;
    async fn history(&self, limit: usize, store: Arc<dyn RunStore>) -> Result<Vec<RunStatus>>;
}

pub(crate) async fn run_http_adapter(
    run_id: RunId,
    store: &Arc<dyn RunStore>,
    api_key: &str,
    api_key_error: &str,
    error_label: &str,
    build_request: impl FnOnce() -> RequestBuilder,
    extract_text: impl FnOnce(&Value) -> Option<String>,
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

    if api_key.is_empty() {
        store
            .update_status(
                run_id,
                RunStatus {
                    state: RunState::Failed,
                    message: Some(api_key_error.into()),
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await?;
        return Err(anyhow!("{}", api_key_error));
    }

    let resp = build_request()
        .send()
        .await
        .with_context(|| format!("calling {} API", error_label))?;

    let status = resp.status();
    let text = resp.text().await?;
    let parsed: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({ "raw": text }));

    if !status.is_success() {
        let msg = parsed
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or(&format!("{} call failed", error_label.to_lowercase()))
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

    let completion = extract_text(&parsed).unwrap_or_else(|| text.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::claude::ClaudeAdapter;
    use crate::backend::codex::CodexAdapter;
    use crate::store::{BackendKind, MemRunStore, RunState};
    use serde_json::json;

    #[tokio::test]
    async fn codex_capabilities_and_run_flow() {
        let adapter = CodexAdapter::new("gpt-5-codex".into()).unwrap();
        assert_eq!(adapter.backend(), BackendKind::Codex);
        let caps = adapter.capabilities();
        assert!(caps.supports_schema);
        assert!(caps.supports_async);
        assert!(caps.supports_tracing);
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = adapter
            .run(
                RunRequest {
                    backend: BackendKind::Codex,
                    prompt: "hello".into(),
                    template_id: None,
                    output_schema: Some(json!({"type": "object"})),
                    async_mode: false,
                    tracing: true,
                },
                store.clone(),
            )
            .await
            .unwrap();
        let status = adapter
            .status(run_id, store.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.state, RunState::Running);
        let hist = adapter.history(5, store.clone()).await.unwrap();
        assert_eq!(hist.len(), 1);
    }

    #[tokio::test]
    async fn claude_capabilities_and_run_flow() {
        let adapter = ClaudeAdapter::new("claude-code".into()).unwrap();
        assert_eq!(adapter.backend(), BackendKind::Claude);
        let caps = adapter.capabilities();
        assert!(caps.supports_schema);
        assert!(caps.supports_async);
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = adapter
            .run(
                RunRequest {
                    backend: BackendKind::Claude,
                    prompt: "hi".into(),
                    template_id: Some("default".into()),
                    output_schema: None,
                    async_mode: false,
                    tracing: false,
                },
                store.clone(),
            )
            .await
            .unwrap();
        let status = adapter
            .status(run_id, store.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.state, RunState::Running);
        let stopped = adapter.stop(run_id, store.clone()).await.unwrap();
        assert!(stopped);
        let status = adapter
            .status(run_id, store.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.state, RunState::Canceled);
    }
}
