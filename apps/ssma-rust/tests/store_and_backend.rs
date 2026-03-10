use anyhow::Result;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

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

#[derive(Clone, Default)]
struct RequestCapture {
    payload: Arc<Mutex<Option<Value>>>,
}

async fn capture_apply(
    State(capture): State<RequestCapture>,
    Json(body): Json<Value>,
) -> Json<Value> {
    *capture.payload.lock().expect("capture lock") = Some(body);
    Json(json!({ "results": [] }))
}

#[tokio::test]
async fn backend_client_uses_canonical_context_shape() -> Result<()> {
    let capture = RequestCapture::default();
    let app = Router::new()
        .route("/apply-intents", post(capture_apply))
        .with_state(capture.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let client = ssma_rust::backend::BackendHttpClient::new(format!("http://127.0.0.1:{}", addr.port()));
    let context = ssma_rust::backend::BackendContext {
        site: "default".to_string(),
        connection_id: Some("conn-1".to_string()),
        ip: Some("127.0.0.1".to_string()),
        user_agent: Some("cargo-test".to_string()),
        user: Some(ssma_rust::backend::BackendUser {
            id: Some("user-1".to_string()),
            role: "staff".to_string(),
        }),
    };

    client
        .apply_intents(
            vec![json!({
                "id": "intent-1",
                "intent": "TODO_CREATE",
                "payload": { "id": "todo-1" },
                "meta": {}
            })],
            &context,
        )
        .await?;

    let payload = capture
        .payload
        .lock()
        .expect("capture lock")
        .clone()
        .expect("captured payload");
    assert_eq!(
        payload.get("context"),
        Some(&json!({
            "site": "default",
            "connectionId": "conn-1",
            "ip": "127.0.0.1",
            "userAgent": "cargo-test",
            "user": {
                "id": "user-1",
                "role": "staff"
            }
        }))
    );

    handle.abort();
    Ok(())
}
