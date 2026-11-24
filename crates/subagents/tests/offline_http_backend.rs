//! With `SKRILLS_OFFLINE` set, the HTTP backends refuse a run instead of
//! calling the API. Its own test binary, because the variable is process-wide
//! and would refuse the unit tests' mock-server runs.

use std::sync::Arc;
use std::time::Duration;

use skrills_subagents::backend::codex::CodexAdapter;
use skrills_subagents::backend::config::AdapterConfig;
use skrills_subagents::backend::BackendAdapter;
use skrills_subagents::store::MemRunStore;
use skrills_subagents::{BackendKind, RunRequest, RunState, RunStore};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn an_http_backend_refuses_to_run_when_the_offline_switch_is_on() {
    std::env::set_var("SKRILLS_OFFLINE", "1");
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let adapter = CodexAdapter::with_config(AdapterConfig {
        api_key: "test-key".into(),
        base_url: reqwest::Url::parse(&format!("{}/v1", server.uri())).unwrap(),
        model: "gpt-test".into(),
        timeout: Duration::from_secs(10),
    })
    .unwrap();
    let store: Arc<dyn RunStore> = Arc::new(MemRunStore::new());
    let request = RunRequest {
        backend: BackendKind::Codex,
        prompt: "hello".into(),
        template_id: None,
        output_schema: None,
        async_mode: false,
        tracing: false,
        model: None,
    };

    let run_id = adapter.run(request, store.clone()).await.unwrap();

    let mut status = store.run(run_id).await.unwrap().unwrap().status;
    for _ in 0..200 {
        if !matches!(status.state, RunState::Pending | RunState::Running) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        status = store.run(run_id).await.unwrap().unwrap().status;
    }
    assert_eq!(status.state, RunState::Failed, "{status:?}");
    let message = status.message.unwrap_or_default();
    assert!(message.contains("offline mode"), "{message}");
}
