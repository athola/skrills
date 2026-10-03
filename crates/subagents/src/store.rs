use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
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

impl RunState {
    /// Whether the run has finished. A finished run never changes state again.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunState::Succeeded | RunState::Failed | RunState::Canceled
        )
    }
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

/// Applies `status` unless the run already finished.
///
/// A run that was stopped (`Canceled`) must not flip to `Succeeded` when its
/// still-running task completes a moment later, so a terminal state is final.
/// Returns whether the status changed.
fn apply_status(record: &mut RunRecord, status: RunStatus) -> bool {
    if record.status.state.is_terminal() {
        tracing::debug!(
            run_id = %record.id,
            current = ?record.status.state,
            ignored = ?status.state,
            "ignoring status change for a finished run"
        );
        return false;
    }
    record.updated_at = status.updated_at;
    record.status = status;
    true
}

/// `kind` of the event that stands in for events dropped at the per-run cap.
/// Its `data` is `{"count": n}`: the run's first `n` events are gone.
pub(crate) const EVENTS_DROPPED_KIND: &str = "events_dropped";

/// How many of the run's events were dropped at the cap: the count carried
/// by a leading `events_dropped` marker, or 0 when there is none.
pub(crate) fn dropped_event_count(events: &[RunEvent]) -> usize {
    events
        .first()
        .filter(|e| e.kind == EVENTS_DROPPED_KIND)
        .and_then(|e| e.data.as_ref()?.get("count")?.as_u64())
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(0)
}

/// Appends `event`, keeping at most [`MAX_EVENTS_PER_RUN`] events. Past the
/// cap the oldest events are replaced by one `events_dropped` marker counting
/// every event dropped so far, so an event's position in the run never
/// changes: event `n` stays event `n` (RT-31).
fn push_event(record: &mut RunRecord, event: RunEvent) {
    record.updated_at = event.ts;
    record.events.push(event);
    if record.events.len() <= MAX_EVENTS_PER_RUN {
        return;
    }
    let already_dropped = dropped_event_count(&record.events);
    let marker_slots = usize::from(already_dropped > 0);
    // Keep room for the marker itself.
    let excess = record.events.len() - (MAX_EVENTS_PER_RUN - 1);
    let dropping = excess - marker_slots;
    let last_dropped_ts = record.events[excess - 1].ts;
    record.events.drain(..excess);
    record.events.insert(
        0,
        RunEvent {
            ts: last_dropped_ts,
            kind: EVENTS_DROPPED_KIND.to_string(),
            data: Some(serde_json::json!({ "count": already_dropped + dropping })),
        },
    );
}

/// Drops the oldest finished runs once more than [`MAX_RUNS`] are held.
/// Unfinished runs are never dropped.
fn prune_runs(runs: &mut HashMap<RunId, RunRecord>) {
    if runs.len() <= MAX_RUNS {
        return;
    }
    let mut finished: Vec<(OffsetDateTime, RunId)> = runs
        .values()
        .filter(|r| r.status.state.is_terminal())
        .map(|r| (r.created_at, r.id))
        .collect();
    finished.sort_by_key(|(created_at, _)| *created_at);
    let excess = runs.len() - MAX_RUNS;
    for (_, id) in finished.into_iter().take(excess) {
        runs.remove(&id);
    }
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
        prune_runs(&mut guard);
        Ok(id)
    }

    async fn update_status(&self, run_id: RunId, status: RunStatus) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let record = guard
            .get_mut(&run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        apply_status(record, status);
        Ok(())
    }

    async fn append_event(&self, run_id: RunId, event: RunEvent) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let record = guard
            .get_mut(&run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        push_event(record, event);
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

/// Maximum events kept per run to bound memory usage for long-running subagents.
const MAX_EVENTS_PER_RUN: usize = 10_000;

/// Maximum runs kept; the oldest finished runs are dropped beyond this.
const MAX_RUNS: usize = 500;

/// Message given to runs found unfinished when the store is opened.
const INTERRUPTED_MESSAGE: &str = "interrupted: the server stopped before the run finished";

/// Disk-backed store using the shared state directory.
pub struct StateRunStore {
    path: PathBuf,
    inner: Arc<Mutex<HashMap<RunId, RunRecord>>>,
    /// Held across snapshot and write so concurrent persists land in order.
    persist_lock: Mutex<()>,
}

impl StateRunStore {
    /// Opens the store at `path`.
    ///
    /// Runs left unfinished by an earlier process are marked `Failed`, since
    /// nothing is left to finish them. A file that does not parse is moved
    /// aside as `<name>.corrupt-<unix time>` and the store starts empty, so
    /// one truncated write does not disable the subagent service.
    pub fn new(path: PathBuf) -> Result<Self> {
        let records = match read_records(&path) {
            Ok(records) => records,
            Err(ReadError::Io(e)) => {
                return Err(e).with_context(|| {
                    format!("failed to initialize store from: {}", path.display())
                })
            }
            Err(ReadError::Parse(e)) => {
                let aside = quarantine(&path)?;
                tracing::warn!(
                    path = %path.display(),
                    moved_to = %aside.display(),
                    error = %e,
                    "subagent run store did not parse; moved it aside and started empty"
                );
                Vec::new()
            }
        };
        let now = OffsetDateTime::now_utc();
        let mut runs = HashMap::new();
        for mut record in records {
            apply_status(
                &mut record,
                RunStatus {
                    state: RunState::Failed,
                    message: Some(INTERRUPTED_MESSAGE.into()),
                    updated_at: now,
                },
            );
            runs.insert(record.id, record);
        }
        Ok(Self {
            path,
            inner: Arc::new(Mutex::new(runs)),
            persist_lock: Mutex::new(()),
        })
    }

    /// Reload the in-memory store from disk, waiting for the lock.
    pub async fn load_from_disk(&self) -> Result<()> {
        let path = self.path.clone();
        let records = tokio::task::spawn_blocking(move || read_records(&path))
            .await
            .context("store reload task panicked")?
            .map_err(ReadError::into_anyhow)
            .with_context(|| format!("failed to reload store from: {}", self.path.display()))?;
        let mut guard = self.inner.lock().await;
        guard.clear();
        for record in records {
            guard.insert(record.id, record);
        }
        Ok(())
    }

    async fn persist(&self) -> Result<()> {
        // Snapshot and write under one lock: two persists that snapshot in one
        // order and rename in the other would leave the older state on disk.
        let _persisting = self.persist_lock.lock().await;
        let runs: Vec<RunRecord> = {
            let guard = self.inner.lock().await;
            let mut runs: Vec<_> = guard.values().cloned().collect();
            runs.sort_by_key(|r| r.created_at);
            runs
        };
        let data =
            serde_json::to_string_pretty(&runs).context("failed to serialize run records")?;

        // Move filesystem I/O to the blocking threadpool to avoid starving
        // the tokio async runtime under load.
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || write_private_atomic(&path, data.as_bytes()))
            .await
            .context("persist task panicked")?
    }
}

/// Writes `data` to `path` through a uniquely named temp file in the same
/// directory, then renames it into place. The temp file is created owner-only
/// (0600 on unix), so prompts and outputs are not readable by other users.
fn write_private_atomic(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create store directory: {}", parent.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("failed to create temp file in: {}", parent.display()))?;
    temp.write_all(data)
        .with_context(|| format!("failed to write temp file: {}", temp.path().display()))?;
    temp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("failed to rename temp file to: {}", path.display()))?;
    Ok(())
}

/// Moves an unreadable store file aside and returns where it went.
fn quarantine(path: &Path) -> Result<PathBuf> {
    let stamp = OffsetDateTime::now_utc().unix_timestamp();
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".corrupt-{stamp}"));
    let aside = path.with_file_name(name);
    fs::rename(path, &aside).with_context(|| {
        format!(
            "failed to move unreadable store {} aside to {}",
            path.display(),
            aside.display()
        )
    })?;
    Ok(aside)
}

/// Default on-disk path for persisted runs.
pub fn default_store_path() -> Result<PathBuf> {
    Ok(home_dir()?.join(".codex/subagents/runs.json"))
}

enum ReadError {
    Io(anyhow::Error),
    Parse(anyhow::Error),
}

impl ReadError {
    fn into_anyhow(self) -> anyhow::Error {
        match self {
            ReadError::Io(e) | ReadError::Parse(e) => e,
        }
    }
}

fn read_records(path: &Path) -> std::result::Result<Vec<RunRecord>, ReadError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read store file: {}", path.display()))
        .map_err(ReadError::Io)?;
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse store file: {}", path.display()))
        .map_err(ReadError::Parse)
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
            prune_runs(&mut guard);
        }
        self.persist().await?;
        Ok(id)
    }

    async fn update_status(&self, run_id: RunId, status: RunStatus) -> Result<()> {
        let changed = {
            let mut guard = self.inner.lock().await;
            let record = guard
                .get_mut(&run_id)
                .ok_or(SubagentError::NotFound(run_id))?;
            apply_status(record, status)
        };
        if changed {
            self.persist().await?;
        }
        Ok(())
    }

    async fn append_event(&self, run_id: RunId, event: RunEvent) -> Result<()> {
        // Streamed tokens and lines are kept in memory and written with the
        // next status change or non-stream event: persisting each one rewrote
        // every run once per token.
        let persist = event.kind != "stream";
        {
            let mut guard = self.inner.lock().await;
            let record = guard
                .get_mut(&run_id)
                .ok_or(SubagentError::NotFound(run_id))?;
            push_event(record, event);
        }
        if persist {
            self.persist().await?;
        }
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

        // Stop should persist a canceled status.
        let stopped = store.stop(run_id).await.unwrap();
        assert!(stopped);

        // Reopen store to ensure persistence was written.
        let reopened = StateRunStore::new(path.clone()).unwrap();
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
        let result = tokio::time::timeout(Duration::from_secs(1), load_task)
            .await
            .expect("load_from_disk should complete within timeout");
        result
            .expect("load task should not panic")
            .expect("load_from_disk should succeed");

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

    /// RT-16: a store file that does not parse is moved aside, not fatal.
    #[tokio::test]
    async fn test_state_store_corrupted_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        fs::write(&path, "{ corrupted json }").unwrap();

        let store = StateRunStore::new(path.clone()).expect("a corrupt file must not be fatal");
        assert!(store.history(10).await.unwrap().is_empty());
        assert!(!path.exists());
        let aside: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("runs.json.corrupt-"))
            .collect();
        assert_eq!(aside.len(), 1, "the bad file should be kept for inspection");
        assert_eq!(
            fs::read_to_string(dir.path().join(&aside[0])).unwrap(),
            "{ corrupted json }"
        );
    }

    /// RT-7: a finished run never changes state again.
    #[tokio::test]
    async fn test_canceled_run_is_not_flipped_to_succeeded() {
        let dir = tempfile::tempdir().unwrap();
        for store in [
            Arc::new(MemRunStore::new()) as Arc<dyn RunStore>,
            Arc::new(StateRunStore::new(dir.path().join("runs.json")).unwrap()),
        ] {
            let run_id = store.create_run(sample_request()).await.unwrap();
            assert!(store.stop(run_id).await.unwrap());
            store
                .update_status(
                    run_id,
                    RunStatus {
                        state: RunState::Succeeded,
                        message: Some("completed".into()),
                        updated_at: OffsetDateTime::now_utc(),
                    },
                )
                .await
                .unwrap();
            let status = store.status(run_id).await.unwrap().unwrap();
            assert_eq!(status.state, RunState::Canceled);
        }
    }

    /// RT-15: runs left unfinished on disk are failed on load.
    #[tokio::test]
    async fn test_unfinished_runs_are_failed_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = StateRunStore::new(path.clone()).unwrap();
        let running = store.create_run(sample_request()).await.unwrap();
        store
            .update_status(
                running,
                RunStatus {
                    state: RunState::Running,
                    message: None,
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await
            .unwrap();
        let done = store.create_run(sample_request()).await.unwrap();
        store.stop(done).await.unwrap();

        let reopened = StateRunStore::new(path).unwrap();
        let status = reopened.status(running).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Failed);
        assert_eq!(status.message.as_deref(), Some(INTERRUPTED_MESSAGE));
        let status = reopened.status(done).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Canceled);
    }

    /// RT-14: concurrent writes all land on disk.
    #[tokio::test]
    async fn test_concurrent_creates_all_persist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = Arc::new(StateRunStore::new(path.clone()).unwrap());
        let mut tasks = Vec::new();
        for _ in 0..40 {
            let store = store.clone();
            tasks.push(tokio::spawn(async move {
                store.create_run(sample_request()).await
            }));
        }
        for task in tasks {
            task.await.unwrap().expect("concurrent persist failed");
        }
        let reopened = StateRunStore::new(path).unwrap();
        assert_eq!(reopened.history(100).await.unwrap().len(), 40);
        let leftovers = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1, "temp files must not be left behind");
    }

    /// RT-13: streamed events are not written one by one.
    #[tokio::test]
    async fn test_stream_events_are_persisted_with_the_next_status_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = StateRunStore::new(path.clone()).unwrap();
        let run_id = store.create_run(sample_request()).await.unwrap();
        let stream = |n: usize| RunEvent {
            ts: OffsetDateTime::now_utc(),
            kind: "stream".into(),
            data: Some(Value::from(n)),
        };
        store.append_event(run_id, stream(1)).await.unwrap();
        store.append_event(run_id, stream(2)).await.unwrap();
        assert!(
            !fs::read_to_string(&path).unwrap().contains("\"stream\""),
            "stream events should not trigger a write"
        );

        store
            .update_status(
                run_id,
                RunStatus {
                    state: RunState::Succeeded,
                    message: None,
                    updated_at: OffsetDateTime::now_utc(),
                },
            )
            .await
            .unwrap();
        let on_disk = fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk.matches("\"stream\"").count(), 2);
        // In memory the events were there all along.
        assert_eq!(store.run(run_id).await.unwrap().unwrap().events.len(), 2);
    }

    /// RT-33: the oldest finished runs are pruned past the cap.
    #[tokio::test]
    async fn test_runs_are_capped() {
        let store = MemRunStore::new();
        let first = store.create_run(sample_request()).await.unwrap();
        store.stop(first).await.unwrap();
        let unfinished = store.create_run(sample_request()).await.unwrap();
        for _ in 0..MAX_RUNS {
            let id = store.create_run(sample_request()).await.unwrap();
            store.stop(id).await.unwrap();
        }
        assert_eq!(store.history(usize::MAX).await.unwrap().len(), MAX_RUNS);
        assert!(
            store.run(first).await.unwrap().is_none(),
            "oldest finished run dropped"
        );
        assert!(
            store.run(unfinished).await.unwrap().is_some(),
            "unfinished run kept"
        );
    }

    /// RT-34: the store file is owner-only.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_store_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = StateRunStore::new(path.clone()).unwrap();
        store.create_run(sample_request()).await.unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
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
