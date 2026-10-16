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
    /// Model an API run asks for instead of the adapter's default, such as
    /// a named agent's `model`. Absent from records written before it
    /// existed, hence the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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
    /// The process that started the run. Every skrills server (one per
    /// client) shares the store, so a server opening it uses this to leave
    /// alone the runs another live server is still driving. Absent from
    /// records written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<RunOwner>,
}

/// The process that owns a run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunOwner {
    /// Process id of the owning server.
    pub pid: u32,
    /// Random id drawn once per process, so a later process that is given
    /// the same pid (a container's pid 1 after a restart) is not mistaken
    /// for the owner.
    pub process: Uuid,
}

impl RunOwner {
    /// This process.
    pub fn current() -> Self {
        static PROCESS: std::sync::OnceLock<Uuid> = std::sync::OnceLock::new();
        Self {
            pid: std::process::id(),
            process: *PROCESS.get_or_init(Uuid::new_v4),
        }
    }

    /// Whether the owning process may still finish the run. A failed check
    /// counts as alive: failing a run another server is driving loses work,
    /// leaving a dead one Running only leaves a stale entry.
    fn is_alive(&self) -> bool {
        let me = Self::current();
        if self.pid == me.pid {
            return self.process == me.process;
        }
        pid_is_alive(self.pid)
    }
}

/// Whether a process with this pid exists, by `kill -0`. A pid owned by
/// another user also reads as gone; the store lives in the user's home.
#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    match std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(e) => {
            tracing::warn!(pid, error = %e, "could not check whether a run's owner is alive; assuming it is");
            true
        }
    }
}

/// No portable check without new dependencies; an unknown owner counts as
/// alive, so its runs are left as they are.
#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool {
    true
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
/// changes: event `n` stays event `n` (RT-31). Eviction drops
/// [`EVENT_EVICTION_BLOCK`] events at once, so the held events are shifted
/// once per block rather than on every push.
fn push_event(record: &mut RunRecord, event: RunEvent) {
    // A canceled run's task may still be finishing; what it reports after
    // the run ended is not part of the run.
    if record.status.state.is_terminal() {
        tracing::debug!(
            run_id = %record.id,
            kind = %event.kind,
            "ignoring event for a finished run"
        );
        return;
    }
    record.updated_at = event.ts;
    record.events.push(event);
    if record.events.len() <= MAX_EVENTS_PER_RUN {
        return;
    }
    let already_dropped = dropped_event_count(&record.events);
    let marker_slots = usize::from(already_dropped > 0);
    // Keep room for the marker itself.
    let excess = record.events.len() - (MAX_EVENTS_PER_RUN - EVENT_EVICTION_BLOCK);
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

/// Marks an unfinished run canceled; shared by both stores so they cannot
/// drift. Returns whether it changed.
fn stop_record(record: &mut RunRecord) -> bool {
    if record.status.state.is_terminal() {
        return false;
    }
    let now = OffsetDateTime::now_utc();
    record.status = RunStatus {
        state: RunState::Canceled,
        message: Some("stopped by user".into()),
        updated_at: now,
    };
    record.updated_at = now;
    true
}

/// The `limit` newest runs, newest first. Ordered by reference and cloned
/// only after truncating, so a small limit does not copy every run.
fn newest_runs(runs: &HashMap<RunId, RunRecord>, limit: usize) -> Vec<RunRecord> {
    let mut newest: Vec<&RunRecord> = runs.values().collect();
    newest.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    newest.truncate(limit);
    newest.into_iter().cloned().collect()
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
            owner: Some(RunOwner::current()),
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
        Ok(newest_runs(&guard, limit))
    }

    async fn stop(&self, run_id: RunId) -> Result<bool> {
        let mut guard = self.inner.lock().await;
        let record = guard
            .get_mut(&run_id)
            .ok_or(SubagentError::NotFound(run_id))?;
        Ok(stop_record(record))
    }
}

/// Maximum events kept per run to bound memory usage for long-running subagents.
const MAX_EVENTS_PER_RUN: usize = 10_000;

/// Events dropped at once when a run passes [`MAX_EVENTS_PER_RUN`].
const EVENT_EVICTION_BLOCK: usize = MAX_EVENTS_PER_RUN / 10;

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
    /// Unfinished runs whose owner is gone (or unrecorded) are marked
    /// `Failed` and written back, since nothing is left to finish them; runs
    /// another live server owns are left alone. A file that does not parse
    /// is moved aside as `<name>.corrupt-<unix time>` and the store starts
    /// empty, so one truncated write does not disable the subagent service.
    pub fn new(path: PathBuf) -> Result<Self> {
        let _lock = lock_store(&path)?;
        let records = read_records_or_quarantine(&path)
            .with_context(|| format!("failed to initialize store from: {}", path.display()))?;
        let now = OffsetDateTime::now_utc();
        let mut runs = HashMap::new();
        let mut interrupted = false;
        for mut record in records {
            // Finished runs are skipped first: each liveness check spawns `kill`.
            if !record.status.state.is_terminal()
                && !record.owner.is_some_and(|owner| owner.is_alive())
            {
                interrupted |= apply_status(
                    &mut record,
                    RunStatus {
                        state: RunState::Failed,
                        message: Some(INTERRUPTED_MESSAGE.into()),
                        updated_at: now,
                    },
                );
            }
            runs.insert(record.id, record);
        }
        if interrupted {
            write_runs(&path, &runs)?;
        }
        Ok(Self {
            path,
            inner: Arc::new(Mutex::new(runs)),
            persist_lock: Mutex::new(()),
        })
    }

    /// Reload the in-memory store from disk, waiting for the lock.
    ///
    /// Holds the persist lock throughout, so a persist cannot land between
    /// the read and the swap and be overwritten by the older file. Events
    /// held only in memory (unpersisted stream lines) are replaced by
    /// what is on disk.
    pub async fn load_from_disk(&self) -> Result<()> {
        let _persisting = self.persist_lock.lock().await;
        let path = self.path.clone();
        let records = tokio::task::spawn_blocking(move || {
            let _lock = lock_store(&path).map_err(ReadError::Io)?;
            read_records(&path)
        })
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
        let ours = self.inner.lock().await.clone();
        // Other servers write the same file, so the write merges into what
        // is on disk now, read under the cross-process lock, instead of
        // replacing it. Serialization and filesystem I/O run on the blocking
        // threadpool so a large store does not stall the async workers.
        let path = self.path.clone();
        let merged = tokio::task::spawn_blocking(move || {
            let _lock = lock_store(&path)?;
            let on_disk = read_records_or_quarantine(&path)?;
            let merged = merge_runs(on_disk.into_iter().map(|r| (r.id, r)).collect(), ours);
            write_runs(&path, &merged)?;
            Ok::<_, anyhow::Error>(merged)
        })
        .await
        .context("persist task panicked")??;
        // Runs changed in memory since the snapshot are kept by the merge
        // rule; what other servers wrote becomes visible here.
        let mut guard = self.inner.lock().await;
        let current = std::mem::take(&mut *guard);
        *guard = merge_runs(merged, current);
        Ok(())
    }
}

/// Merges `ours` (this store's memory) into `theirs` (the file as another
/// server may have left it). A finished run is final, so a finished copy
/// beats an unfinished one; otherwise the copy from the owning process
/// wins, and runs this process does not own are taken as written.
fn merge_runs(
    mut theirs: HashMap<RunId, RunRecord>,
    ours: HashMap<RunId, RunRecord>,
) -> HashMap<RunId, RunRecord> {
    let me = RunOwner::current();
    for (id, record) in ours {
        let keep_ours = match theirs.get(&id) {
            None => true,
            Some(other) => match (
                record.status.state.is_terminal(),
                other.status.state.is_terminal(),
            ) {
                (true, false) => true,
                (false, true) => false,
                _ => record.owner == Some(me),
            },
        };
        if keep_ours {
            theirs.insert(id, record);
        }
    }
    prune_runs(&mut theirs);
    theirs
}

/// Opens and exclusively locks `<store>.lock` beside the store file,
/// blocking until no other process holds it; dropping the file releases it.
/// A separate file, because every write renames a new store file into place
/// and a lock on the replaced one would guard nothing.
fn lock_store(path: &Path) -> Result<fs::File> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    let lock_path = path.with_file_name(name);
    if let Some(parent) = lock_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create store directory: {}", parent.display()))?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("failed to open store lock: {}", lock_path.display()))?;
    file.lock()
        .with_context(|| format!("failed to lock store: {}", lock_path.display()))?;
    Ok(file)
}

/// Writes `runs` oldest first as compact JSON (the file is rewritten on
/// every status change).
fn write_runs(path: &Path, runs: &HashMap<RunId, RunRecord>) -> Result<()> {
    let mut sorted: Vec<&RunRecord> = runs.values().collect();
    sorted.sort_by_key(|r| r.created_at);
    let data = serde_json::to_vec(&sorted).context("failed to serialize run records")?;
    write_private_atomic(path, &data)
}

/// Reads the store; a file that does not parse is moved aside and read as
/// empty, as at startup, so it cannot block every later write.
fn read_records_or_quarantine(path: &Path) -> Result<Vec<RunRecord>> {
    match read_records(path) {
        Ok(records) => Ok(records),
        Err(ReadError::Io(e)) => Err(e),
        Err(ReadError::Parse(e)) => {
            let aside = quarantine(path)?;
            tracing::warn!(
                path = %path.display(),
                moved_to = %aside.display(),
                error = %e,
                "subagent run store did not parse; moved it aside and started empty"
            );
            Ok(Vec::new())
        }
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
    // Data reaches the disk before the rename does; otherwise a crash can
    // leave a renamed but empty file, which is then quarantined at startup.
    temp.as_file()
        .sync_all()
        .with_context(|| format!("failed to sync temp file: {}", temp.path().display()))?;
    temp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("failed to rename temp file to: {}", path.display()))?;
    // The rename itself is durable once the directory entry is synced.
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|dir| dir.sync_all())
        .with_context(|| format!("failed to sync store directory: {}", parent.display()))?;
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
            owner: Some(RunOwner::current()),
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
        Ok(newest_runs(&guard, limit))
    }

    async fn stop(&self, run_id: RunId) -> Result<bool> {
        {
            let mut guard = self.inner.lock().await;
            let record = guard
                .get_mut(&run_id)
                .ok_or(SubagentError::NotFound(run_id))?;
            if !stop_record(record) {
                return Ok(false);
            }
        }
        self.persist().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// M257: past the cap, eviction drops a block of old events at once, so
    /// the held events are not shifted on every later push.
    #[test]
    fn events_past_the_cap_are_evicted_in_blocks_not_per_push() {
        let now = OffsetDateTime::now_utc();
        let mut record = RunRecord {
            id: RunId(Uuid::new_v4()),
            request: super::store_tests::sample_request(),
            status: RunStatus {
                state: RunState::Running,
                message: None,
                updated_at: now,
            },
            events: Vec::new(),
            created_at: now,
            updated_at: now,
            owner: Some(RunOwner::current()),
        };
        let event = |n: usize| RunEvent {
            ts: now,
            kind: "tick".into(),
            data: Some(serde_json::json!({ "n": n })),
        };
        for n in 0..=MAX_EVENTS_PER_RUN {
            push_event(&mut record, event(n));
        }
        let first_held = record.events[1].clone();
        for n in 0..10 {
            push_event(&mut record, event(MAX_EVENTS_PER_RUN + 1 + n));
        }
        assert_eq!(record.events[1], first_held);
        assert!(record.events.len() <= MAX_EVENTS_PER_RUN);
        // The marker still counts exactly the events no longer held.
        let first_n = record.events[1].data.as_ref().unwrap()["n"]
            .as_u64()
            .unwrap();
        assert_eq!(dropped_event_count(&record.events) as u64, first_n);
    }

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
                model: None,
            })
            .await
            .unwrap();
        let second = store
            .create_run(RunRequest {
                backend: BackendKind::Claude,
                prompt: "second".into(),
                template_id: None,
                output_schema: None,
                async_mode: false,
                tracing: false,
                model: None,
            })
            .await
            .unwrap();

        // Explicit timestamps instead of sleeping between the two creates.
        {
            let mut runs = store.inner.lock().await;
            let base = OffsetDateTime::now_utc();
            runs.get_mut(&first).unwrap().created_at = base - time::Duration::seconds(1);
            runs.get_mut(&second).unwrap().created_at = base;
        }
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
                model: None,
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

    /// M264: a finished run takes no further events.
    #[tokio::test]
    async fn events_for_a_finished_run_are_ignored() {
        let store = MemRunStore::new();
        let run_id = store.create_run(sample_request()).await.unwrap();
        assert!(store.stop(run_id).await.unwrap());
        store
            .append_event(
                run_id,
                RunEvent {
                    ts: OffsetDateTime::now_utc(),
                    kind: "completion".into(),
                    data: None,
                },
            )
            .await
            .unwrap();
        assert!(store.run(run_id).await.unwrap().unwrap().events.is_empty());
    }

    /// M258: runs.json is written compact, not pretty-printed.
    #[tokio::test]
    async fn state_store_writes_compact_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let store = StateRunStore::new(path.clone()).unwrap();
        store.create_run(sample_request()).await.unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains('\n'), "{text}");
        let runs: Vec<RunRecord> = serde_json::from_str(&text).unwrap();
        assert_eq!(runs.len(), 1);
    }

    pub(super) fn sample_request() -> RunRequest {
        RunRequest {
            backend: BackendKind::Codex,
            prompt: "hello".to_string(),
            template_id: Some("default".to_string()),
            output_schema: None,
            async_mode: false,
            tracing: false,
            model: None,
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

    /// RT-15, narrowed by M260: a reopen fails only runs nobody can finish.
    #[tokio::test]
    async fn runs_of_a_live_owner_survive_a_reopen() {
        // Was `test_unfinished_runs_are_failed_on_reopen`: reopening failed
        // every unfinished run. Under M260 a run whose owner (here, this
        // process) is alive is left alone; a dead owner's run is covered by
        // `startup_interrupts_only_runs_whose_owner_is_gone`.
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
        assert_eq!(status.state, RunState::Running);
        let status = reopened.status(done).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Canceled);
    }

    /// M260: two servers (one per client) share runs.json; each persist
    /// rewrote the whole file from its own memory, so the last writer
    /// dropped the other's runs.
    #[tokio::test]
    async fn two_stores_on_one_file_keep_each_others_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let first = StateRunStore::new(path.clone()).unwrap();
        let second = StateRunStore::new(path.clone()).unwrap();

        let from_first = first.create_run(sample_request()).await.unwrap();
        let from_second = second.create_run(sample_request()).await.unwrap();
        first
            .append_event(
                from_first,
                RunEvent {
                    ts: OffsetDateTime::now_utc(),
                    kind: "progress".into(),
                    data: None,
                },
            )
            .await
            .unwrap();

        let on_disk: Vec<RunId> = read_records(&path)
            .map_err(ReadError::into_anyhow)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert!(on_disk.contains(&from_first), "{on_disk:?}");
        assert!(on_disk.contains(&from_second), "{on_disk:?}");
    }

    /// M260: a stop recorded by a store that does not own the run is final;
    /// the owner's next write must not put the run back to Running.
    #[tokio::test]
    async fn a_stop_from_another_store_survives_the_owners_next_write() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let owner = StateRunStore::new(path.clone()).unwrap();
        let run = owner.create_run(sample_request()).await.unwrap();
        let other = StateRunStore::new(path.clone()).unwrap();
        assert!(other.stop(run).await.unwrap());

        owner
            .append_event(
                run,
                RunEvent {
                    ts: OffsetDateTime::now_utc(),
                    kind: "progress".into(),
                    data: None,
                },
            )
            .await
            .unwrap();

        let reopened = StateRunStore::new(path).unwrap();
        let status = reopened.status(run).await.unwrap().unwrap();
        assert_eq!(status.state, RunState::Canceled);
        assert_eq!(
            owner.status(run).await.unwrap().unwrap().state,
            RunState::Canceled,
            "the owner's own view picks up the stop on its next write"
        );
    }

    /// M260: opening the store failed every unfinished run, including runs
    /// another live server was still driving. Only runs nobody can finish
    /// are interrupted: a dead owner, an earlier process that had this pid,
    /// or a record written before owners were kept.
    #[cfg(unix)]
    #[tokio::test]
    async fn startup_interrupts_only_runs_whose_owner_is_gone() {
        let mut sleeper = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let mut exited = std::process::Command::new("true").spawn().unwrap();
        exited.wait().unwrap();

        let now = OffsetDateTime::now_utc();
        let running = |owner: Option<RunOwner>| RunRecord {
            id: RunId(Uuid::new_v4()),
            request: sample_request(),
            status: RunStatus {
                state: RunState::Running,
                message: None,
                updated_at: now,
            },
            events: Vec::new(),
            created_at: now,
            updated_at: now,
            owner,
        };
        let live = running(Some(RunOwner {
            pid: sleeper.id(),
            process: Uuid::new_v4(),
        }));
        let dead = running(Some(RunOwner {
            pid: exited.id(),
            process: Uuid::new_v4(),
        }));
        let earlier_self = running(Some(RunOwner {
            pid: std::process::id(),
            process: Uuid::new_v4(),
        }));
        let legacy = running(None);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.json");
        let records = vec![
            live.clone(),
            dead.clone(),
            earlier_self.clone(),
            legacy.clone(),
        ];
        fs::write(&path, serde_json::to_vec(&records).unwrap()).unwrap();

        let store = {
            // The liveness check runs `kill` from PATH, which other tests
            // replace while they hold this guard.
            let _env = skrills_test_utils::env_guard();
            StateRunStore::new(path.clone()).unwrap()
        };
        let state = |id| {
            let store = &store;
            async move { store.status(id).await.unwrap().unwrap().state }
        };
        let live_state = state(live.id).await;
        let dead_state = state(dead.id).await;
        let earlier_self_state = state(earlier_self.id).await;
        let legacy_state = state(legacy.id).await;
        sleeper.kill().unwrap();
        sleeper.wait().unwrap();

        assert_eq!(live_state, RunState::Running);
        assert_eq!(dead_state, RunState::Failed);
        assert_eq!(earlier_self_state, RunState::Failed);
        assert_eq!(legacy_state, RunState::Failed);
        let on_disk = read_records(&path).map_err(ReadError::into_anyhow).unwrap();
        let dead_on_disk = on_disk.iter().find(|r| r.id == dead.id).unwrap();
        assert_eq!(
            dead_on_disk.status.state,
            RunState::Failed,
            "the interruption is written, so other servers see it"
        );
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
        // The store file and its lock file, which is kept between writes.
        let leftovers = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 2, "temp files must not be left behind");
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
