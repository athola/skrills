pub mod claude;
pub mod codex;
pub mod config;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::store::{BackendKind, RunId, RunRequest, RunStatus, RunStore, SubagentTemplate};

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
    async fn get_status(
        &self,
        run_id: RunId,
        store: Arc<dyn RunStore>,
    ) -> Result<Option<RunStatus>>;
    async fn stop(&self, run_id: RunId, store: Arc<dyn RunStore>) -> Result<bool>;
    async fn history(&self, limit: usize, store: Arc<dyn RunStore>) -> Result<Vec<RunStatus>>;
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
        let adapter = CodexAdapter::new("gpt-5-codex".into());
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
            .get_status(run_id, store.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.state, RunState::Running);
        let hist = adapter.history(5, store.clone()).await.unwrap();
        assert_eq!(hist.len(), 1);
    }

    #[tokio::test]
    async fn claude_capabilities_and_run_flow() {
        let adapter = ClaudeAdapter::new("claude-code".into());
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
            .get_status(run_id, store.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.state, RunState::Running);
        let stopped = adapter.stop(run_id, store.clone()).await.unwrap();
        assert!(stopped);
        let status = adapter
            .get_status(run_id, store.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.state, RunState::Canceled);
    }
}
