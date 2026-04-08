use anyhow::Result;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{header, HeaderValue, Response, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use bytes::Bytes;
use futures_util::StreamExt;
use serde_json::{json, Value};
use ssma_rust::config::Config;
use ssma_rust::gateway;
use std::path::Path as StdPath;
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

fn test_config(tmp: &StdPath, backend_url: String) -> Config {
    let mut config = Config::from_env();
    config.auth_jwt_secret = "test-secret-key-for-e2e".to_string();
    config.auth_cookie_secure = false;
    config.user_store_path = tmp.join("users.json");
    config.intent_store_path = tmp.join("intents.json");
    config.media_storage_root = tmp.join("media");
    config.backend_url = backend_url;
    config
}

async fn spawn_gateway(config: Config) -> Result<(String, tokio::task::JoinHandle<()>)> {
    let state = gateway::build_state(config);
    let app = gateway::app(state);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("127.0.0.1:{}", addr.port()), handle))
}

fn ndjson_response(parts: Vec<&'static str>) -> Response<Body> {
    let stream = async_stream::stream! {
        for part in parts {
            yield Ok::<Bytes, std::convert::Infallible>(Bytes::from(part.to_string()));
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };

    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    response
}

async fn backend_stream(Path(name): Path<String>, Json(body): Json<Value>) -> Response<Body> {
    assert_eq!(name, "ai.generate");
    assert_eq!(body.get("stream").and_then(Value::as_bool), Some(true));

    match body
        .get("payload")
        .and_then(|v| v.get("case"))
        .and_then(Value::as_str)
    {
        Some("single-chunk") => ndjson_response(vec![
            "{\"type\":\"chunk\",\"delta\":\"Hello\"}\n{\"type\":\"chunk\",\"delta\":\" world\"}\n{\"type\":\"done\"}\n",
        ]),
        Some("split-line") => ndjson_response(vec![
            "{\"type\":\"chunk\",\"del",
            "ta\":\"Hello\"}\n",
            "{\"type\":\"chunk\",\"delta\":\" world\"}\n{\"type\":\"done\"}\n",
        ]),
        Some("malformed") => ndjson_response(vec![
            "{\"type\":\"chunk\",\"delta\":\"Hello\"}\n",
            "{\"type\":\"chunk\",\"delta\":}\n",
        ]),
        _ => ndjson_response(vec![]),
    }
}

async fn spawn_backend() -> Result<(String, tokio::task::JoinHandle<()>)> {
    let app = Router::new().route("/query/:name", post(backend_stream));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok((format!("http://127.0.0.1:{}", addr.port()), handle))
}

async fn collect_sse_frames(response: reqwest::Response) -> Result<Vec<(String, Value)>> {
    let mut frames = Vec::new();
    let mut stream = response.bytes_stream();
    let mut buf = String::new();

    loop {
        let next = timeout(Duration::from_millis(500), stream.next()).await;
        match next {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(split) = buf.find("\n\n") {
                    let frame = buf[..split].to_string();
                    buf = buf[split + 2..].to_string();
                    let mut event_name = "message".to_string();
                    let mut data = String::new();
                    for line in frame.lines() {
                        if let Some(value) = line.strip_prefix("event:") {
                            event_name = value.trim().to_string();
                        }
                        if let Some(value) = line.strip_prefix("data:") {
                            data.push_str(value.trim());
                        }
                    }
                    let parsed = serde_json::from_str::<Value>(&data)
                        .unwrap_or_else(|_| json!({ "raw": data }));
                    frames.push((event_name, parsed));
                }
            }
            Ok(Some(Err(error))) => return Err(error.into()),
            Ok(None) => break,
            Err(_) => break,
        }
    }

    Ok(frames)
}

#[tokio::test]
async fn query_stream_forwards_multiple_ndjson_objects_in_one_chunk() -> Result<()> {
    let (backend_base, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let (gateway_base, gateway_handle) = spawn_gateway(test_config(tmp.path(), backend_base)).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/query/ai.generate/stream", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({ "payload": { "case": "single-chunk" } }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let frames = collect_sse_frames(response).await?;
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].1["type"], "chunk");
    assert_eq!(frames[0].1["delta"], "Hello");
    assert_eq!(frames[1].1["delta"], " world");
    assert_eq!(frames[2].1["type"], "done");

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn query_stream_reassembles_split_ndjson_lines() -> Result<()> {
    let (backend_base, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let (gateway_base, gateway_handle) = spawn_gateway(test_config(tmp.path(), backend_base)).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/query/ai.generate/stream", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({ "payload": { "case": "split-line" } }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let frames = collect_sse_frames(response).await?;
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].1["delta"], "Hello");
    assert_eq!(frames[1].1["delta"], " world");
    assert_eq!(frames[2].1["type"], "done");

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn query_stream_sets_anon_cookie_when_missing() -> Result<()> {
    let (backend_base, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let (gateway_base, gateway_handle) = spawn_gateway(test_config(tmp.path(), backend_base)).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/query/ai.generate/stream", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({ "payload": { "case": "single-chunk" } }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let set_cookie = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("ssma_anon="))
        .expect("expected ssma_anon cookie");
    assert!(set_cookie.contains("HttpOnly"));

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}

#[tokio::test]
async fn query_stream_emits_error_event_on_malformed_ndjson() -> Result<()> {
    let (backend_base, backend_handle) = spawn_backend().await?;
    let tmp = tempfile::tempdir()?;
    let (gateway_base, gateway_handle) = spawn_gateway(test_config(tmp.path(), backend_base)).await?;

    let response = reqwest::Client::new()
        .post(format!("http://{}/query/ai.generate/stream", gateway_base))
        .header("content-type", "application/json")
        .json(&json!({ "payload": { "case": "malformed" } }))
        .send()
        .await?;

    assert_eq!(response.status(), StatusCode::OK);
    let frames = collect_sse_frames(response).await?;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].1["type"], "chunk");
    assert_eq!(frames[1].0, "error");
    assert_eq!(frames[1].1["error"], "STREAM_ERROR");

    gateway_handle.abort();
    backend_handle.abort();
    Ok(())
}
