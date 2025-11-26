use skrills_state::{auto_pin_from_history, load_history, save_history, HistoryEntry};
use tempfile::tempdir;

#[test]
fn history_round_trip_and_autopin() {
    let tmp = tempdir().unwrap();
    std::env::set_var("HOME", tmp.path());
    let history = vec![HistoryEntry {
        ts: 1,
        skills: vec!["a".into(), "b".into(), "a".into(), "b".into()],
    }];
    save_history(history.clone()).unwrap();
    let loaded = load_history().unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].skills, history[0].skills);

    let auto = auto_pin_from_history(&loaded);
    assert!(auto.contains("a"));
    assert!(auto.contains("b"));
}
