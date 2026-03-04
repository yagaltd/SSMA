use anyhow::Result;

#[test]
fn intent_store_assigns_monotonic_cursor() -> Result<()> {
    let path = std::env::temp_dir().join(format!("ssma-rust-store-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = ssma_rust::runtime::IntentStore::new(path.clone(), 300_000);
    let now = ssma_rust::runtime::now_millis();
    let appended = store.append_batch(vec![
        ssma_rust::runtime::IntentRecord {
            id: "r-1".to_string(),
            intent: "TODO_CREATE".to_string(),
            payload: serde_json::json!({"id":"a"}),
            meta: serde_json::json!({"clock": 1}),
            inserted_at: now,
            log_seq: 0,
            site: "default".to_string(),
            status: "acked".to_string(),
            connection_id: None,
            backend: None,
        },
        ssma_rust::runtime::IntentRecord {
            id: "r-2".to_string(),
            intent: "TODO_CREATE".to_string(),
            payload: serde_json::json!({"id":"b"}),
            meta: serde_json::json!({"clock": 2}),
            inserted_at: now + 1,
            log_seq: 0,
            site: "default".to_string(),
            status: "acked".to_string(),
            connection_id: None,
            backend: None,
        },
    ]);
    assert_eq!(appended.fresh.len(), 2);
    assert!(appended.fresh[0].log_seq < appended.fresh[1].log_seq);
    assert_eq!(store.latest_cursor(), appended.fresh[1].log_seq);
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn intent_store_dedupes_by_site_and_id() -> Result<()> {
    let path = std::env::temp_dir().join(format!("ssma-rust-store-dedupe-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let store = ssma_rust::runtime::IntentStore::new(path.clone(), 300_000);
    let now = ssma_rust::runtime::now_millis();
    let one = ssma_rust::runtime::IntentRecord {
        id: "dup-1".to_string(),
        intent: "TODO_CREATE".to_string(),
        payload: serde_json::json!({"id":"a"}),
        meta: serde_json::json!({"clock": 1}),
        inserted_at: now,
        log_seq: 0,
        site: "s1".to_string(),
        status: "acked".to_string(),
        connection_id: None,
        backend: None,
    };
    let two = ssma_rust::runtime::IntentRecord {
        id: "dup-1".to_string(),
        intent: "TODO_CREATE".to_string(),
        payload: serde_json::json!({"id":"a2"}),
        meta: serde_json::json!({"clock": 2}),
        inserted_at: now + 1,
        log_seq: 0,
        site: "s1".to_string(),
        status: "acked".to_string(),
        connection_id: None,
        backend: None,
    };
    let three = ssma_rust::runtime::IntentRecord {
        site: "s2".to_string(),
        ..two.clone()
    };

    let first = store.append_batch(vec![one]);
    let second = store.append_batch(vec![two]);
    let third = store.append_batch(vec![three]);

    assert_eq!(first.fresh.len(), 1);
    assert_eq!(second.fresh.len(), 0);
    assert_eq!(third.fresh.len(), 1);
    let _ = std::fs::remove_file(path);
    Ok(())
}
