use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use skrills_state::home_dir;
use std::fmt;
use time::OffsetDateTime;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum BackendKind {
    Codex,
    Claude,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentTemplate {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub backend: BackendKind,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunRequest {
    pub backend: BackendKind,
    pub prompt: String,
    pub template_id: Option<String>,
    pub output_schema: Option<Value>,
    pub async_mode: bool,
    pub tracing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunStatus {
    pub state: RunState,
    pub message: Option<String>,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunEvent {
    pub ts: OffsetDateTime,
    pub kind: String,
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunRecord {
    pub id: RunId,
    pub request: RunRequest,
    pub status: RunStatus,
    pub events: Vec<RunEvent>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct RunId(pub Uuid);

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SubagentError {
    #[error("run not found: {0}")]
    NotFound(RunId),
    #[error("run already completed: {0}")]
    Completed(RunId),
    #[error("storage error: {0}")]
    Storage(String),
}

#[async_trait]
pub trait RunStore: Send + Sync {
    async fn create_run(&self, request: RunRequest) -> Result<RunId>;
    async fn update_status(&self, run_id: RunId, status: RunStatus) -> Result<()>;
    async fn append_event(&self, run_id: RunId, event: RunEvent) -> Result<()>;
    async fn run(&self, run_id: RunId) -> Result<Option<RunRecord>>;
    async fn status(&self, run_id: RunId) -> Result<Option<RunStatus>>;
    async fn history(&self, limit: usize) -> Result<Vec<RunRecord>>;
    async fn stop(&self, run_id: RunId) -> Result<bool>;
}

/// In-memory store for tests and ephemeral runs.
pub struct MemRunStore {
    inner: Arc<Mutex<HashMap<RunId, RunRecord>>>,
}

impl Default for MemRunStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemRunStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl RunStore for MemRunStore {
    async fn create_run(&self, request: RunRequest) -> Result<RunId> {
        let now = OffsetDateTime::now_utc();
        let id = RunId(Uuid::new_v4());
        let record = RunRecord {
            id,
            request,
            status: RunStatus {
                state: RunState::Pending,
                message: None,
                updated_at: now,
            },
            events: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        let mut guard = self.inner.lock().await;
        guard.insert(id, record);
        Ok(id)
    }

    async fn update_status(&self, run_id: RunId, status: RunStatus) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let record = guard
            .get_mut(&run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        record.status = status.clone();
        record.updated_at = status.updated_at;
        Ok(())
    }

    async fn append_event(&self, run_id: RunId, event: RunEvent) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let record = guard
            .get_mut(&run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        record.updated_at = event.ts;
        record.events.push(event);
        Ok(())
    }

    async fn run(&self, run_id: RunId) -> Result<Option<RunRecord>> {
        let guard = self.inner.lock().await;
        Ok(guard.get(&run_id).cloned())
    }

    async fn status(&self, run_id: RunId) -> Result<Option<RunStatus>> {
        let guard = self.inner.lock().await;
        Ok(guard.get(&run_id).map(|r| r.status.clone()))
    }

    async fn history(&self, limit: usize) -> Result<Vec<RunRecord>> {
        let guard = self.inner.lock().await;
        let mut runs: Vec<_> = guard.values().cloned().collect();
        runs.sort_by_key(|r| r.created_at);
        runs.reverse();
        runs.truncate(limit);
        Ok(runs)
    }

    async fn stop(&self, run_id: RunId) -> Result<bool> {
        let mut guard = self.inner.lock().await;
        let record = guard
            .get_mut(&run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        match record.status.state {
            RunState::Succeeded | RunState::Failed | RunState::Canceled => Ok(false),
            _ => {
                let now = OffsetDateTime::now_utc();
                record.status = RunStatus {
                    state: RunState::Canceled,
                    message: Some("stopped by user".into()),
                    updated_at: now,
                };
                record.updated_at = now;
                Ok(true)
            }
        }
    }
}

/// Disk-backed store using the shared state directory.
pub struct StateRunStore {
    path: PathBuf,
    inner: Arc<Mutex<HashMap<RunId, RunRecord>>>,
}

impl StateRunStore {
    pub fn new(path: PathBuf) -> Result<Self> {
        let records = read_records(&path)
            .with_context(|| format!("failed to initialize store from: {}", path.display()))?;
        let mut runs = HashMap::new();
        for record in records {
            runs.insert(record.id, record);
        }
        Ok(Self {
            path,
            inner: Arc::new(Mutex::new(runs)),
        })
    }

    /// Reload the in-memory store from disk, waiting for the lock.
    pub async fn load_from_disk(&self) -> Result<()> {
        let records = read_records(&self.path)
            .with_context(|| format!("failed to reload store from: {}", self.path.display()))?;
        let mut guard = self.inner.lock().await;
        guard.clear();
        for record in records {
            guard.insert(record.id, record);
        }
        Ok(())
    }

    async fn persist(&self) -> Result<()> {
        let runs: Vec<RunRecord> = {
            let guard = self.inner.lock().await;
            let mut runs: Vec<_> = guard.values().cloned().collect();
            runs.sort_by_key(|r| r.created_at);
            runs
        };
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create store directory: {}", parent.display())
            })?;
        }
        let data =
            serde_json::to_string_pretty(&runs).context("failed to serialize run records")?;

        // Atomic write: write to temp file then rename to avoid partial writes on crash.
        let temp_path = self.path.with_extension("tmp");
        fs::write(&temp_path, &data)
            .with_context(|| format!("failed to write temp file: {}", temp_path.display()))?;
        fs::rename(&temp_path, &self.path)
            .with_context(|| format!("failed to rename temp file to: {}", self.path.display()))?;
        Ok(())
    }
}

/// Default on-disk path for persisted runs.
pub fn default_store_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".codex/subagents/runs.json"))
}

fn read_records(path: &PathBuf) -> Result<Vec<RunRecord>> {
    if path.exists() {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read store file: {}", path.display()))?;
        let records: Vec<RunRecord> = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse store file: {}", path.display()))?;
        Ok(records)
    } else {
        Ok(Vec::new())
    }
}

#[async_trait]
impl RunStore for StateRunStore {
    async fn create_run(&self, request: RunRequest) -> Result<RunId> {
        let now = OffsetDateTime::now_utc();
        let id = RunId(Uuid::new_v4());
        let record = RunRecord {
            id,
            request,
            status: RunStatus {
                state: RunState::Pending,
                message: None,
                updated_at: now,
            },
            events: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        {
            let mut guard = self.inner.lock().await;
            guard.insert(id, record);
        }
        self.persist().await?;
        Ok(id)
    }

    async fn update_status(&self, run_id: RunId, status: RunStatus) -> Result<()> {
        {
            let mut guard = self.inner.lock().await;
            let record = guard
                .get_mut(&run_id)
                .ok_or(SubagentError::NotFound(run_id))?;
            record.status = status.clone();
            record.updated_at = status.updated_at;
        }
        self.persist().await?;
        Ok(())
    }

    async fn append_event(&self, run_id: RunId, event: RunEvent) -> Result<()> {
        {
            let mut guard = self.inner.lock().await;
            let record = guard
                .get_mut(&run_id)
                .ok_or(SubagentError::NotFound(run_id))?;
            record.updated_at = event.ts;
            record.events.push(event);
        }
        self.persist().await?;
        Ok(())
    }

    async fn run(&self, run_id: RunId) -> Result<Option<RunRecord>> {
        let guard = self.inner.lock().await;
        Ok(guard.get(&run_id).cloned())
    }

    async fn status(&self, run_id: RunId) -> Result<Option<RunStatus>> {
        let guard = self.inner.lock().await;
        Ok(guard.get(&run_id).map(|r| r.status.clone()))
    }

    async fn history(&self, limit: usize) -> Result<Vec<RunRecord>> {
        let guard = self.inner.lock().await;
        let mut runs: Vec<_> = guard.values().cloned().collect();
        runs.sort_by_key(|r| r.created_at);
        runs.reverse();
        runs.truncate(limit);
        Ok(runs)
    }

    async fn stop(&self, run_id: RunId) -> Result<bool> {
        {
            let mut guard = self.inner.lock().await;
            let record = guard
                .get_mut(&run_id)
                .ok_or(SubagentError::NotFound(run_id))?;
            match record.status.state {
                RunState::Succeeded | RunState::Failed | RunState::Canceled => return Ok(false),
                _ => {
                    let now = OffsetDateTime::now_utc();
                    record.status = RunStatus {
                        state: RunState::Canceled,
                        message: Some("stopped by user".into()),
                        updated_at: now,
                    };
                    record.updated_at = now;
                }
            }
        }
        self.persist().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn given_new_store_when_create_run_then_history_returns_most_recent_first() {
        let store = MemRunStore::new();
        let first = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "first".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
            })
            .await
            .unwrap();
        // ensure distinct timestamps for deterministic ordering
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        let second = store
            .create_run(RunRequest {
                backend: BackendKind::Claude,
                prompt: "second".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
            })
            .await
            .unwrap();

        let history = store.history(10).await.unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history.first().unwrap().id, second);
        assert_eq!(history.last().unwrap().id, first);
    }

    #[tokio::test]
    async fn given_running_run_when_stop_invoked_then_status_becomes_canceled() {
        let store = MemRunStore::new();
        let run_id = store
            .create_run(RunRequest {
                backend: BackendKind::Codex,
                prompt: "stop me".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
            })
            .await
            .unwrap();

        let stopped = store.stop(run_id).await.unwrap();
        assert!(stopped);

        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Canceled);
        assert_eq!(status.message.as_deref(), Some("stopped by user"));
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;
    use tokio::time::Duration;

    fn sample_request() -> RunRequest {
        RunRequest {
            backend: BackendKind::Codex,
            prompt: "hello".to_string(),
            template_id: Some("default".to_string()),
            output_schema: None,
            async_mode: false,
            tracing: false,
        }
    }

    #[tokio::test]
    async fn given_mem_store_when_create_run_then_status_is_pending() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();
        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Pending);
    }

    #[tokio::test]
    async fn given_mem_store_when_updating_status_and_appending_event_then_persists_in_memory() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();
        let new_status = RunStatus {
            state: RunState::Running,
            message: Some("working".into()),
            updated_at: OffsetDateTime::now_utc(),
        };
        store
            .update_status(run_id, new_status.clone())
            .await
            .unwrap();
        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Running);

        let event = RunEvent {
            ts: OffsetDateTime::now_utc(),
            kind: "progress".into(),
            data: Some(Value::String("step1".into())),
        };
        store.append_event(run_id, event.clone()).await.unwrap();
        let history = store.history(10).await.unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].events.len(), 1);
        assert_eq!(history[0].events[0].kind, "progress");
    }

    #[tokio::test]
    async fn given_mem_store_when_stop_then_marks_canceled() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();
        let stopped = store.stop(run_id).await.unwrap();
        assert!(stopped);
        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Canceled);
    }

    #[tokio::test]
    async fn given_state_store_when_reopened_then_status_and_history_persist_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = StateRunStore::new(path.clone()).unwrap();
        let run_id = store.create_run(sample_request()).await.unwrap();
        let status = RunStatus {
            state: RunState::Running,
            message: Some("working".into()),
            updated_at: OffsetDateTime::now_utc(),
        };
        store.update_status(run_id, status.clone()).await.unwrap();

        // Reopen store to ensure persistence was written.
        let reopened = StateRunStore::new(path.clone()).unwrap();
        let got = reopened.status(run_id).await.unwrap().unwrap();
        assert_eq!(got.state, status.state);

        // Stop should persist a canceled status.
        let stopped = reopened.stop(run_id).await.unwrap();
        assert!(stopped);
        let got = reopened.status(run_id).await.unwrap().unwrap();
        assert_eq!(got.state, RunState::Canceled);

        // History should include the run.
        let hist = reopened.history(10).await.unwrap();
        assert_eq!(hist.len(), 1);
    }

    #[tokio::test]
    async fn given_state_store_when_load_contended_then_waits_for_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        fs::write(&path, "[]").unwrap();

        let store = Arc::new(StateRunStore::new(path).unwrap());
        let inner = store.inner.clone();

        let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let lock_task = tokio::spawn(async move {
            let _guard = inner.lock().await;
            let _ = locked_tx.send(());
            let _ = release_rx.await;
        });

        locked_rx.await.unwrap();

        let store_clone = Arc::clone(&store);
        let load_task = tokio::spawn(async move { store_clone.load_from_disk().await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!load_task.is_finished());

        let _ = release_tx.send(());
        let result = tokio::time::timeout(Duration::from_secs(1), load_task).await;
        assert!(result.is_ok());
        assert!(result.unwrap().unwrap().is_ok());

        let _ = lock_task.await;
    }

    #[tokio::test]
    async fn test_update_status_nonexistent_run_returns_error() {
        let store = MemRunStore::new();
        let fake_id = RunId(uuid::Uuid::new_v4());
        let status = RunStatus {
            state: RunState::Running,
            message: Some("test".into()),
            updated_at: OffsetDateTime::now_utc(),
        };
        let result = store.update_status(fake_id, status).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("run not found"),
            "expected 'run not found' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_append_event_nonexistent_run_returns_error() {
        let store = MemRunStore::new();
        let fake_id = RunId(uuid::Uuid::new_v4());
        let event = RunEvent {
            ts: OffsetDateTime::now_utc(),
            kind: "test".into(),
            data: None,
        };
        let result = store.append_event(fake_id, event).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("run not found"),
            "expected 'run not found' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_stop_already_completed_run_returns_false() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();

        // Mark as succeeded
        store
            .update_status(
                run_id,
                RunStatus {
                    state: RunState::Succeeded,
                    message: Some("done".into()),
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await
            .unwrap();

        // Stop should return false for already-completed runs
        let stopped = store.stop(run_id).await.unwrap();
        assert!(!stopped, "stop() should return false for completed runs");

        // Verify status is still Succeeded (not changed to Canceled)
        let status = store.status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Succeeded);
    }

    #[tokio::test]
    async fn test_stop_already_failed_run_returns_false() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();

        // Mark as failed
        store
            .update_status(
                run_id,
                RunStatus {
                    state: RunState::Failed,
                    message: Some("error".into()),
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await
            .unwrap();

        // Stop should return false for already-failed runs
        let stopped = store.stop(run_id).await.unwrap();
        assert!(!stopped, "stop() should return false for failed runs");
    }

    #[tokio::test]
    async fn test_stop_already_canceled_run_returns_false() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();

        // First stop succeeds
        let first_stop = store.stop(run_id).await.unwrap();
        assert!(first_stop, "first stop() should succeed");

        // Second stop returns false (already canceled)
        let second_stop = store.stop(run_id).await.unwrap();
        assert!(
            !second_stop,
            "stop() should return false for already-canceled runs"
        );
    }

    #[tokio::test]
    async fn test_stop_nonexistent_run_returns_error() {
        let store = MemRunStore::new();
        let fake_id = RunId(uuid::Uuid::new_v4());
        let result = store.stop(fake_id).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("run not found"),
            "expected 'run not found' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_state_store_corrupted_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        fs::write(&path, "{ corrupted json }").unwrap();

        let result = StateRunStore::new(path);
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(
            err.to_string().contains("failed to")
                || err.to_string().contains("parse")
                || err.to_string().contains("invalid"),
            "expected parse/initialization error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_state_store_update_status_nonexistent_run_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = StateRunStore::new(path).unwrap();

        let fake_id = RunId(uuid::Uuid::new_v4());
        let status = RunStatus {
            state: RunState::Running,
            message: Some("test".into()),
            updated_at: OffsetDateTime::now_utc(),
        };
        let result = store.update_status(fake_id, status).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_state_store_append_event_nonexistent_run_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = StateRunStore::new(path).unwrap();

        let fake_id = RunId(uuid::Uuid::new_v4());
        let event = RunEvent {
            ts: OffsetDateTime::now_utc(),
            kind: "test".into(),
            data: None,
        };
        let result = store.append_event(fake_id, event).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_state_store_stop_already_completed_run_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = StateRunStore::new(path).unwrap();
        let run_id = store.create_run(sample_request()).await.unwrap();

        // Mark as succeeded
        store
            .update_status(
                run_id,
                RunStatus {
                    state: RunState::Succeeded,
                    message: Some("done".into()),
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await
            .unwrap();

        // Stop should return false
        let stopped = store.stop(run_id).await.unwrap();
        assert!(!stopped);
    }
}
