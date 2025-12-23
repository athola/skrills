use std::fs;

use skrills_subagents::store::StateRunStore;
use skrills_subagents::{
    BackendKind, RunEvent, RunId, RunRecord, RunRequest, RunState, RunStatus, RunStore,
};
use tempfile::tempdir;
use time::OffsetDateTime;
use uuid::Uuid;

fn sample_record() -> RunRecord {
    let now = OffsetDateTime::now_utc();
    RunRecord {
        id: RunId(Uuid::new_v4()),
        request: RunRequest {
            backend: BackendKind::Codex,
            prompt: "hello".to_string(),
            template_id: None,
            output_schema: None,
            async_mode: false,
            tracing: false,
        },
        status: RunStatus {
            state: RunState::Succeeded,
            message: Some("done".into()),
            updated_at: now,
        },
        events: vec![RunEvent {
            ts: now,
            kind: "start".into(),
            data: None,
        }],
        created_at: now,
        updated_at: now,
    }
}

#[tokio::test]
async fn given_store_when_reloading_from_disk_then_history_updates() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("runs.json");

    let record = sample_record();
    let data = serde_json::to_string_pretty(&vec![record]).unwrap();
    fs::write(&path, data).unwrap();

    let store = StateRunStore::new(path.clone()).unwrap();
    let history = store.history(10).await.unwrap();
    assert_eq!(history.len(), 1);

    fs::write(&path, "[]").unwrap();
    store.load_from_disk().await.unwrap();

    let history = store.history(10).await.unwrap();
    assert!(history.is_empty());
}
