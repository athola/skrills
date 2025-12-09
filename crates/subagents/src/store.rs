use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use skrills_state::home_dir;
use std::fmt;
use std::sync::Mutex;
use time::OffsetDateTime;
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
    async fn get_run(&self, run_id: RunId) -> Result<Option<RunRecord>>;
    async fn get_status(&self, run_id: RunId) -> Result<Option<RunStatus>>;
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
        let mut guard = self.inner.lock().unwrap();
        guard.insert(id, record);
        Ok(id)
    }

    async fn update_status(&self, run_id: RunId, status: RunStatus) -> Result<()> {
        let mut guard = self.inner.lock().unwrap();
        let record = guard
            .get_mut(&run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        record.status = status.clone();
        record.updated_at = status.updated_at;
        Ok(())
    }

    async fn append_event(&self, run_id: RunId, event: RunEvent) -> Result<()> {
        let mut guard = self.inner.lock().unwrap();
        let record = guard
            .get_mut(&run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        record.updated_at = event.ts;
        record.events.push(event);
        Ok(())
    }

    async fn get_run(&self, run_id: RunId) -> Result<Option<RunRecord>> {
        let guard = self.inner.lock().unwrap();
        Ok(guard.get(&run_id).cloned())
    }

    async fn get_status(&self, run_id: RunId) -> Result<Option<RunStatus>> {
        let guard = self.inner.lock().unwrap();
        Ok(guard.get(&run_id).map(|r| r.status.clone()))
    }

    async fn history(&self, limit: usize) -> Result<Vec<RunRecord>> {
        let guard = self.inner.lock().unwrap();
        let mut runs: Vec<_> = guard.values().cloned().collect();
        runs.sort_by_key(|r| r.created_at);
        runs.reverse();
        runs.truncate(limit);
        Ok(runs)
    }

    async fn stop(&self, run_id: RunId) -> Result<bool> {
        let mut guard = self.inner.lock().unwrap();
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
        let mut store = Self {
            path,
            inner: Arc::new(Mutex::new(HashMap::new())),
        };
        store.load_from_disk()?;
        Ok(store)
    }

    fn load_from_disk(&mut self) -> Result<()> {
        let mut guard = self.inner.lock().unwrap();
        guard.clear();
        if self.path.exists() {
            let text = fs::read_to_string(&self.path)?;
            let records: Vec<RunRecord> = serde_json::from_str(&text)?;
            for record in records {
                guard.insert(record.id, record);
            }
        }
        Ok(())
    }

    fn persist(&self) -> Result<()> {
        let guard = self.inner.lock().unwrap();
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut runs: Vec<_> = guard.values().cloned().collect();
        runs.sort_by_key(|r| r.created_at);
        let data = serde_json::to_string_pretty(&runs)?;
        fs::write(&self.path, data)?;
        Ok(())
    }
}

/// Default on-disk path for persisted runs.
pub fn default_store_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".codex/subagents/runs.json"))
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
            let mut guard = self.inner.lock().unwrap();
            guard.insert(id, record);
        }
        self.persist()?;
        Ok(id)
    }

    async fn update_status(&self, run_id: RunId, status: RunStatus) -> Result<()> {
        {
            let mut guard = self.inner.lock().unwrap();
            let record = guard
                .get_mut(&run_id)
                .ok_or(SubagentError::NotFound(run_id))?;
            record.status = status.clone();
            record.updated_at = status.updated_at;
        }
        self.persist()?;
        Ok(())
    }

    async fn append_event(&self, run_id: RunId, event: RunEvent) -> Result<()> {
        {
            let mut guard = self.inner.lock().unwrap();
            let record = guard
                .get_mut(&run_id)
                .ok_or(SubagentError::NotFound(run_id))?;
            record.updated_at = event.ts;
            record.events.push(event);
        }
        self.persist()?;
        Ok(())
    }

    async fn get_run(&self, run_id: RunId) -> Result<Option<RunRecord>> {
        let guard = self.inner.lock().unwrap();
        Ok(guard.get(&run_id).cloned())
    }

    async fn get_status(&self, run_id: RunId) -> Result<Option<RunStatus>> {
        let guard = self.inner.lock().unwrap();
        Ok(guard.get(&run_id).map(|r| r.status.clone()))
    }

    async fn history(&self, limit: usize) -> Result<Vec<RunRecord>> {
        let guard = self.inner.lock().unwrap();
        let mut runs: Vec<_> = guard.values().cloned().collect();
        runs.sort_by_key(|r| r.created_at);
        runs.reverse();
        runs.truncate(limit);
        Ok(runs)
    }

    async fn stop(&self, run_id: RunId) -> Result<bool> {
        {
            let mut guard = self.inner.lock().unwrap();
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
        self.persist()?;
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

        let status = store.get_status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Canceled);
        assert_eq!(status.message.as_deref(), Some("stopped by user"));
    }
}

#[cfg(test)]
mod store_tests {
    use super::*;

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
    async fn mem_store_create_and_get_status() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();
        let status = store.get_status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Pending);
    }

    #[tokio::test]
    async fn mem_store_updates_status_and_events() {
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
        let status = store.get_status(run_id).await.unwrap().unwrap();
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
    async fn mem_store_stop_marks_canceled() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();
        let stopped = store.stop(run_id).await.unwrap();
        assert!(stopped);
        let status = store.get_status(run_id).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Canceled);
    }

    #[tokio::test]
    async fn state_store_persists_runs() {
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
        let got = reopened.get_status(run_id).await.unwrap().unwrap();
        assert_eq!(got.state, status.state);

        // Stop should persist a canceled status.
        let stopped = reopened.stop(run_id).await.unwrap();
        assert!(stopped);
        let got = reopened.get_status(run_id).await.unwrap().unwrap();
        assert_eq!(got.state, RunState::Canceled);

        // History should include the run.
        let hist = reopened.history(10).await.unwrap();
        assert_eq!(hist.len(), 1);
    }
}
