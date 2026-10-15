pub mod claude;
pub mod cli;
pub mod codex;
pub mod config;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use anyhow::{anyhow, Context as _, Result};
use async_trait::async_trait;
use reqwest::RequestBuilder;
use serde_json::{json, Value};
use time::OffsetDateTime;
use tokio::task::AbortHandle;

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

/// Abort handles of live run tasks, so `stop-run` can end an HTTP request in
/// flight instead of letting it run on (and bill) after the run is canceled.
static RUN_TASKS: LazyLock<std::sync::Mutex<HashMap<RunId, AbortHandle>>> =
    LazyLock::new(Default::default);

fn run_tasks() -> std::sync::MutexGuard<'static, HashMap<RunId, AbortHandle>> {
    // The map holds plain handles; a panic elsewhere cannot leave it torn.
    RUN_TASKS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Removes a run's entry from [`RUN_TASKS`] when its task ends, whether it
/// finished, failed, panicked or was aborted.
struct RunTaskEntry(RunId);

impl Drop for RunTaskEntry {
    fn drop(&mut self) {
        run_tasks().remove(&self.0);
    }
}

/// Aborts the background task of `run_id`. Returns whether one was live.
pub(crate) fn abort_run_task(run_id: RunId) -> bool {
    match run_tasks().remove(&run_id) {
        Some(handle) => {
            handle.abort();
            true
        }
        None => false,
    }
}

/// Whether the background task of `run_id` is still running.
#[cfg(test)]
pub(crate) fn run_task_live(run_id: RunId) -> bool {
    run_tasks().contains_key(&run_id)
}

/// Creates a run, marks it `Running`, and drives `execute` on a background task.
///
/// Shared by every adapter so the spawn, error-recording and panic-recovery
/// logic lives in one place. An `Err` from `execute` is recorded as an `error`
/// event plus a `Failed` status; a panic is recorded as `Failed` by a monitor
/// task, so a run never stays orphaned in `Running`.
pub(crate) async fn spawn_run<F, Fut>(
    request: RunRequest,
    store: Arc<dyn RunStore>,
    running_message: &str,
    label: &'static str,
    execute: F,
) -> Result<RunId>
where
    F: FnOnce(RunId, RunRequest, Arc<dyn RunStore>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    let run_id = store.create_run(request.clone()).await?;
    store
        .update_status(
            run_id,
            RunStatus {
                state: RunState::Running,
                message: Some(running_message.into()),
                updated_at: OffsetDateTime::now_utc(),
            },
        )
        .await?;

    let task_store = store.clone();
    // Registered before the task can finish, so its entry is always removed.
    let mut tasks = run_tasks();
    let handle = tokio::spawn(async move {
        let _entry = RunTaskEntry(run_id);
        if let Err(err) = execute(run_id, request, task_store.clone()).await {
            tracing::error!(%run_id, error = %err, "{label} run failed");
            if let Err(e) = task_store
                .append_event(
                    run_id,
                    RunEvent {
                        ts: OffsetDateTime::now_utc(),
                        kind: "error".into(),
                        data: Some(json!({"message": err.to_string()})),
                    },
                )
                .await
            {
                tracing::warn!(error = %e, %run_id, "failed to record error event");
            }
            if let Err(e) = task_store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Failed,
                        message: Some(err.to_string()),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await
            {
                tracing::warn!(error = %e, %run_id, "failed to mark run as failed");
            }
        }
    });
    tasks.insert(run_id, handle.abort_handle());
    drop(tasks);
    tokio::spawn(async move {
        if let Err(join_err) = handle.await {
            if join_err.is_cancelled() {
                tracing::debug!(%run_id, "{label} run task aborted by stop");
                return;
            }
            tracing::error!(%run_id, error = %join_err, "{label} backend task panicked");
            let _ = store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Failed,
                        message: Some("internal error: task panicked".into()),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await;
        }
    });

    Ok(run_id)
}

/// What differs between the HTTP providers apart from the request itself.
pub(crate) struct HttpProvider {
    /// Name used in log and error text, e.g. "Codex".
    pub label: &'static str,
    /// Message recorded when no API key is configured.
    pub api_key_error: &'static str,
    /// The provider's stop reason for a reply cut off at the output cap.
    pub truncated_reason: &'static str,
    pub extract_text: fn(&Value) -> Option<String>,
    pub extract_stop_reason: fn(&Value) -> Option<String>,
}

pub(crate) async fn run_http_adapter(
    run_id: RunId,
    store: &Arc<dyn RunStore>,
    api_key: &str,
    provider: &HttpProvider,
    build_request: impl FnOnce() -> RequestBuilder,
) -> Result<()> {
    let HttpProvider {
        label: error_label,
        api_key_error,
        truncated_reason,
        extract_text,
        extract_stop_reason,
    } = *provider;
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
    let stop_reason = extract_stop_reason(&parsed);
    // A reply cut off at the output cap still succeeds, but says so instead of
    // passing for a complete answer.
    let truncated = stop_reason.as_deref() == Some(truncated_reason);
    // The body arrives whole; the completion event carries it as sent.
    store
        .append_event(
            run_id,
            RunEvent {
                ts: OffsetDateTime::now_utc(),
                kind: "completion".into(),
                data: Some(json!({
                    "text": completion,
                    "stop_reason": stop_reason,
                    "truncated": truncated,
                })),
            },
        )
        .await?;

    store
        .update_status(
            run_id,
            RunStatus {
                state: RunState::Succeeded,
                message: Some(if truncated {
                    format!("completed (truncated: {truncated_reason})")
                } else {
                    "completed".into()
                }),
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
    use crate::backend::config::AdapterConfig;
    use crate::store::{BackendKind, MemRunStore, RunRecord, RunState};
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Config pointing at a local mock server. Built directly rather than from
    /// the environment, so an exported real API key can never cause a network
    /// call from a test (RT-41).
    fn mock_config(base: String, model: &str) -> AdapterConfig {
        AdapterConfig {
            api_key: "test-key".into(),
            base_url: reqwest::Url::parse(&base).unwrap(),
            model: model.into(),
            timeout: Duration::from_secs(10),
        }
    }

    fn request(backend: BackendKind) -> RunRequest {
        RunRequest {
            backend,
            prompt: "hello".into(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
        }
    }

    async fn wait_done(store: &Arc<dyn RunStore>, run_id: RunId) -> RunRecord {
        for _ in 0..200 {
            let run = store.run(run_id).await.unwrap().unwrap();
            if !matches!(run.status.state, RunState::Pending | RunState::Running) {
                return run;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("run did not finish");
    }

    fn completion(run: &RunRecord) -> serde_json::Value {
        run.events
            .iter()
            .find(|e| e.kind == "completion")
            .and_then(|e| e.data.clone())
            .expect("completion event")
    }

    /// RT-6 / RT-29: a base URL without a trailing slash keeps its `/v1`.
    #[tokio::test]
    async fn codex_posts_to_v1_chat_completions_under_a_slashless_base() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "hi there"}, "finish_reason": "stop"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter =
            CodexAdapter::with_config(mock_config(format!("{}/v1", server.uri()), "gpt-test"))
                .unwrap();
        assert_eq!(adapter.backend(), BackendKind::Codex);
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = adapter
            .run(request(BackendKind::Codex), store.clone())
            .await
            .unwrap();

        let run = wait_done(&store, run_id).await;
        assert_eq!(run.status.state, RunState::Succeeded, "{:?}", run.status);
        assert_eq!(completion(&run)["text"], "hi there");
        assert_eq!(completion(&run)["truncated"], false);
    }

    /// RT-6: same for the Claude adapter and `messages`.
    #[tokio::test]
    async fn claude_posts_to_v1_messages_under_a_slashless_base() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .and(header("x-api-key", "test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "hello back"}],
                "stop_reason": "end_turn"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter =
            ClaudeAdapter::with_config(mock_config(format!("{}/v1", server.uri()), "claude-test"))
                .unwrap();
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = adapter
            .run(request(BackendKind::Claude), store.clone())
            .await
            .unwrap();

        let run = wait_done(&store, run_id).await;
        assert_eq!(run.status.state, RunState::Succeeded, "{:?}", run.status);
        assert_eq!(completion(&run)["text"], "hello back");
    }

    /// RT-35: a reply cut off at `max_tokens` says so instead of passing for a
    /// complete answer.
    #[tokio::test]
    async fn claude_reply_cut_at_max_tokens_is_marked_truncated() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "content": [{"type": "text", "text": "partial"}],
                "stop_reason": "max_tokens"
            })))
            .mount(&server)
            .await;

        let adapter =
            ClaudeAdapter::with_config(mock_config(format!("{}/v1/", server.uri()), "claude-test"))
                .unwrap();
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = adapter
            .run(request(BackendKind::Claude), store.clone())
            .await
            .unwrap();

        let run = wait_done(&store, run_id).await;
        assert_eq!(run.status.state, RunState::Succeeded);
        assert_eq!(
            run.status.message.as_deref(),
            Some("completed (truncated: max_tokens)")
        );
        assert_eq!(completion(&run)["truncated"], true);
    }

    /// An API error marks the run failed with the provider's message.
    #[tokio::test]
    async fn api_error_fails_the_run() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error": {"message": "bad request body"}})),
            )
            .mount(&server)
            .await;

        let adapter = CodexAdapter::with_config(mock_config(server.uri(), "gpt-test")).unwrap();
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = adapter
            .run(request(BackendKind::Codex), store.clone())
            .await
            .unwrap();

        let run = wait_done(&store, run_id).await;
        assert_eq!(run.status.state, RunState::Failed);
        assert_eq!(run.status.message.as_deref(), Some("bad request body"));
    }

    /// M264: stopping an HTTP run aborts its request task, and nothing is
    /// appended to the canceled run afterwards; the reply is not split into
    /// per-word stream events.
    #[tokio::test]
    async fn stopping_an_http_run_aborts_its_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({
                        "choices": [{"message": {"content": "late reply"}, "finish_reason": "stop"}]
                    }))
                    .set_delay(Duration::from_millis(1500)),
            )
            .mount(&server)
            .await;
        let adapter = CodexAdapter::with_config(mock_config(server.uri(), "gpt-test")).unwrap();
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = adapter
            .run(request(BackendKind::Codex), store.clone())
            .await
            .unwrap();
        while server
            .received_requests()
            .await
            .unwrap_or_default()
            .is_empty()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(store.stop(run_id).await.unwrap());
        assert!(abort_run_task(run_id), "the run's task was not live");
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!run_task_live(run_id), "the request task is still running");

        tokio::time::sleep(Duration::from_millis(1700)).await;
        let run = store.run(run_id).await.unwrap().unwrap();
        assert_eq!(run.status.state, RunState::Canceled);
        assert!(
            run.events.iter().all(|e| e.kind == "start"),
            "{:?}",
            run.events
        );
    }

    #[tokio::test]
    async fn http_reply_is_not_split_into_word_events() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "choices": [{"message": {"content": "a  b\nc"}, "finish_reason": "stop"}]
            })))
            .mount(&server)
            .await;
        let adapter = CodexAdapter::with_config(mock_config(server.uri(), "gpt-test")).unwrap();
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = adapter
            .run(request(BackendKind::Codex), store.clone())
            .await
            .unwrap();
        let run = wait_done(&store, run_id).await;
        assert!(
            run.events.iter().all(|e| e.kind != "stream"),
            "{:?}",
            run.events
        );
        assert_eq!(completion(&run)["text"], "a  b\nc");
    }

    #[tokio::test]
    async fn claude_stop_cancels_a_pending_run() {
        let adapter =
            ClaudeAdapter::with_config(mock_config("http://127.0.0.1:9/v1/".into(), "claude-test"))
                .unwrap();
        let caps = adapter.capabilities();
        assert!(caps.supports_schema);
        assert!(caps.supports_async);
        let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
        let run_id = store
            .create_run(request(BackendKind::Claude))
            .await
            .unwrap();

        assert!(adapter.stop(run_id, store.clone()).await.unwrap());
        let status = adapter
            .status(run_id, store.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.state, RunState::Canceled);
        assert_eq!(adapter.history(5, store).await.unwrap().len(), 1);
    }
}
