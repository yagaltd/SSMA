use axum::body::Body;
use axum::Router;
use http::header::{COOKIE, SET_COOKIE};
use http::{Request, StatusCode};
use serde_json::{json, Value};
use ssma_rust::config::Config;
use ssma_rust::gateway;
use ssma_rust::runtime::{now_millis, IntentRecord};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

fn test_config(tmp: &Path) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config
}

fn extract_cookie(response: &http::Response<Body>) -> Option<String> {
    response
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            if s.starts_with("ssma_session=") {
                Some(s.to_string())
            } else {
                None
            }
        })
}

fn cookie_header_value(cookie: &str) -> String {
    cookie.split(';').next().unwrap_or("").trim().to_string()
}

fn make_intent(
    id: &str,
    site: &str,
    user_id: Option<&str>,
    reasons: &[&str],
    with_undo: bool,
) -> IntentRecord {
    let mut meta = json!({
        "clock": 1,
        "channels": ["global"],
        "reasons": reasons,
    });
    if with_undo {
        meta["undo"] = json!({
            "intent": "test.action.undo",
            "payload": { "reason": "mistake" }
        });
    }
    IntentRecord {
        id: id.to_string(),
        intent: "test.action".to_string(),
        payload: json!({"value": 42}),
        meta,
        inserted_at: now_millis(),
        log_seq: 0,
        site: site.to_string(),
        status: "acked".to_string(),
        connection_id: None,
        actor_key: user_id.map(|value| format!("user:{}", value)),
        user_id: user_id.map(str::to_string),
        backend: None,
    }
}

async fn register_user(app: &Router, email: &str) -> String {
    let body = serde_json::to_string(&json!({
        "email": email,
        "password": "password123",
        "name": "TestUser"
    }))
    .unwrap();
    let req = Request::builder()
        .method("POST")
        .uri("/auth/register")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    cookie_header_value(&extract_cookie(&resp).unwrap())
}

fn registered_user_id(cookie: &str, secret: &str) -> String {
    #[derive(serde::Deserialize)]
    struct Claims {
        sub: String,
    }

    let mut validation = jsonwebtoken::Validation::default();
    validation.validate_aud = false;
    jsonwebtoken::decode::<Claims>(
        cookie.strip_prefix("ssma_session=").unwrap_or(cookie),
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .unwrap()
    .claims
    .sub
}

fn create_staff_jwt(state: &Arc<gateway::AppState>) -> String {
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde::Serialize;
    use ssma_rust::runtime::now_secs;

    #[derive(Serialize)]
    struct Claims {
        sub: String,
        role: String,
        exp: u64,
    }

    let claims = Claims {
        sub: "staff-user-id".to_string(),
        role: "staff".to_string(),
        exp: now_secs() + 3600,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.auth_jwt_secret.as_bytes()),
    )
    .unwrap()
}

#[tokio::test]
async fn rework_by_staff_user_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    state.store.append_batch(vec![make_intent(
        "intent-1",
        "site-a",
        Some("owner-1"),
        &["pending", "replay", "channel:global"],
        true,
    )]);

    let app = gateway::app(state.clone());
    let staff_cookie = format!("ssma_session={}", create_staff_jwt(&state));

    let req = Request::builder()
        .method("POST")
        .uri("/optimistic/rework")
        .header("content-type", "application/json")
        .header(COOKIE, &staff_cookie)
        .body(Body::from(
            serde_json::to_string(&json!({ "id": "intent-1", "site": "site-a" })).unwrap(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["status"], "scheduled");
    assert_eq!(result["id"], "intent-1");

    let record = state.store.get("intent-1", "site-a").unwrap();
    let reasons = record.meta["reasons"].as_array().unwrap();
    assert!(reasons.iter().any(|value| value == "rework"));
}

#[tokio::test]
async fn rework_rejected_when_stored_undo_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    state.store.append_batch(vec![make_intent(
        "intent-2",
        "site-a",
        Some("owner-1"),
        &["pending", "replay", "channel:global"],
        false,
    )]);

    let app = gateway::app(state.clone());
    let staff_cookie = format!("ssma_session={}", create_staff_jwt(&state));

    let req = Request::builder()
        .method("POST")
        .uri("/optimistic/rework")
        .header("content-type", "application/json")
        .header(COOKIE, &staff_cookie)
        .body(Body::from(
            serde_json::to_string(&json!({ "id": "intent-2", "site": "site-a" })).unwrap(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn undo_by_owner_succeeds_and_clears_reasons() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state.clone());
    let user_cookie = register_user(&app, "undo-user@example.com").await;
    let user_id = registered_user_id(&user_cookie, &state.config.auth_jwt_secret);
    state.store.append_batch(vec![make_intent(
        "intent-3",
        "site-b",
        Some(&user_id),
        &["pending", "replay", "rework", "channel:global"],
        true,
    )]);

    let req = Request::builder()
        .method("POST")
        .uri("/optimistic/undo")
        .header("content-type", "application/json")
        .header(COOKIE, &user_cookie)
        .body(Body::from(
            serde_json::to_string(&json!({
                "id": "intent-3",
                "site": "site-b",
                "intent": "test.action.undo",
                "payload": { "reason": "mistake" }
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(result["status"], "reverted");
    assert_eq!(result["id"], "intent-3");

    let record = state.store.get("intent-3", "site-b").unwrap();
    assert_eq!(record.status, "undone");
    assert_eq!(record.meta["reasons"], json!([]));
}

#[tokio::test]
async fn undo_rejected_for_non_owner_when_owner_metadata_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state.clone());
    let user_cookie = register_user(&app, "someone-else@example.com").await;

    state.store.append_batch(vec![make_intent(
        "intent-4",
        "site-a",
        Some("different-owner"),
        &["pending", "replay", "channel:global"],
        true,
    )]);

    let req = Request::builder()
        .method("POST")
        .uri("/optimistic/undo")
        .header("content-type", "application/json")
        .header(COOKIE, &user_cookie)
        .body(Body::from(
            serde_json::to_string(&json!({
                "id": "intent-4",
                "site": "site-a",
                "intent": "test.action.undo",
                "payload": { "reason": "mistake" }
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn undo_rejected_for_payload_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state.clone());
    let user_cookie = register_user(&app, "legacy-undo@example.com").await;

    state.store.append_batch(vec![make_intent(
        "intent-5",
        "site-a",
        None,
        &["pending", "replay", "channel:global"],
        true,
    )]);

    let req = Request::builder()
        .method("POST")
        .uri("/optimistic/undo")
        .header("content-type", "application/json")
        .header(COOKIE, &user_cookie)
        .body(Body::from(
            serde_json::to_string(&json!({
                "id": "intent-5",
                "site": "site-a",
                "intent": "test.action.undo",
                "payload": { "reason": "wrong" }
            }))
            .unwrap(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn pending_returns_entries_with_reasons() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);

    state.store.append_batch(vec![
        make_intent("pending-1", "site-x", Some("u1"), &["pending", "channel:global"], true),
        make_intent("pending-2", "site-x", Some("u1"), &["rework", "channel:global"], true),
        make_intent("pending-3", "site-y", Some("u2"), &["pending"], true),
    ]);

    let app = gateway::app(state);
    let user_cookie = register_user(&app, "pending-user@example.com").await;

    let req = Request::builder()
        .method("GET")
        .uri("/optimistic/pending?site=site-x")
        .header(COOKIE, &user_cookie)
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let result: Value = serde_json::from_slice(&body).unwrap();
    let pending = result["pending"].as_array().unwrap();
    assert_eq!(pending.len(), 2);
    assert!(pending.iter().all(|entry| entry["site"] == "site-x"));
    assert!(pending.iter().all(|entry| entry["reasons"].is_array()));
}

#[tokio::test]
async fn pending_rejected_for_guest() {
    let tmp = tempfile::tempdir().unwrap();
    let config = test_config(tmp.path());
    let state = gateway::build_state(config);
    let app = gateway::app(state);

    let req = Request::builder()
        .method("GET")
        .uri("/optimistic/pending")
        .body(Body::empty())
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
