use futures_util::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

/// A streaming response from the backend (NDJSON)
pub type BackendStream = Pin<Box<dyn Stream<Item = Result<Value, String>> + Send>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendUser {
    pub id: Option<String>,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendContext {
    pub site: String,
    pub actor_key: Option<String>,
    pub connection_id: Option<String>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
    pub user: Option<BackendUser>,
}

#[derive(Debug, Clone)]
pub struct BackendHttpClient {
    pub base_url: String,
    client: reqwest::Client,
}

impl BackendHttpClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.base_url.is_empty()
    }

    pub async fn apply_intents(
        &self,
        intents: Vec<Value>,
        context: &BackendContext,
    ) -> Result<Value, reqwest::Error> {
        if !self.is_configured() {
            return Ok(serde_json::json!({ "results": [] }));
        }
        self.post_json(
            "/apply-intents",
            serde_json::json!({
                "intents": intents,
                "context": context
            }),
        )
        .await
    }

    pub async fn query(
        &self,
        name: &str,
        payload: Value,
        context: &BackendContext,
    ) -> Result<Value, reqwest::Error> {
        if !self.is_configured() {
            return Ok(serde_json::json!({ "status": "ok", "data": Value::Null }));
        }
        self.post_json(
            &format!("/query/{}", urlencoding::encode(name)),
            serde_json::json!({ "payload": payload, "context": context }),
        )
        .await
    }

    pub async fn subscribe(
        &self,
        channel: &str,
        params: Value,
        context: &BackendContext,
    ) -> Result<Value, reqwest::Error> {
        if !self.is_configured() {
            return Ok(serde_json::json!({ "status": "ok", "snapshot": [], "cursor": 0 }));
        }
        self.post_json(
            "/subscribe",
            serde_json::json!({ "channel": channel, "params": params, "context": context }),
        )
        .await
    }

    pub async fn health(&self, context: &BackendContext) -> Result<Value, reqwest::Error> {
        if !self.is_configured() {
            return Ok(serde_json::json!({ "status": "ok", "backend": "unconfigured" }));
        }
        self.post_json("/health", serde_json::json!({ "context": context }))
            .await
    }

    pub fn ws_url(&self, path: &str) -> String {
        if let Some(rest) = self.base_url.strip_prefix("https://") {
            format!("wss://{}{}", rest.trim_end_matches('/'), path)
        } else if let Some(rest) = self.base_url.strip_prefix("http://") {
            format!("ws://{}{}", rest.trim_end_matches('/'), path)
        } else {
            format!("ws://{}{}", self.base_url.trim_end_matches('/'), path)
        }
    }

    pub async fn create_audio_session(&self, payload: Value) -> Result<Value, reqwest::Error> {
        self.post_json("/internal/audio/sessions", payload).await
    }

    pub async fn get_audio_session(&self, session_id: &str) -> Result<Value, reqwest::Error> {
        let url = format!(
            "{}/internal/audio/sessions/{}",
            self.base_url,
            urlencoding::encode(session_id)
        );
        let response = self.client.get(url).send().await?;
        response.json::<Value>().await
    }

    pub async fn delete_audio_session(&self, session_id: &str) -> Result<Value, reqwest::Error> {
        let url = format!(
            "{}/internal/audio/sessions/{}",
            self.base_url,
            urlencoding::encode(session_id)
        );
        let response = self.client.delete(url).send().await?;
        response.json::<Value>().await
    }

    pub async fn command_audio_session(
        &self,
        session_id: &str,
        payload: Value,
    ) -> Result<Value, reqwest::Error> {
        self.post_json(
            &format!(
                "/internal/audio/sessions/{}/commands",
                urlencoding::encode(session_id)
            ),
            payload,
        )
        .await
    }

    async fn post_json(&self, path: &str, payload: Value) -> Result<Value, reqwest::Error> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.post(url).json(&payload).send().await?;
        response.json::<Value>().await
    }

    /// Stream NDJSON from backend (for AI streaming, real-time queries)
    pub async fn query_stream(
        &self,
        name: &str,
        payload: Value,
        context: &BackendContext,
    ) -> Result<BackendStream, reqwest::Error> {
        if !self.is_configured() {
            let stream = futures_util::stream::empty();
            return Ok(Box::pin(stream));
        }
        let url = format!(
            "{}/query/{}",
            self.base_url,
            urlencoding::encode(name)
        );
        let response = self
            .client
            .post(&url)
            .header("Accept", "application/x-ndjson")
            .json(&serde_json::json!({ "payload": payload, "context": context, "stream": true }))
            .send()
            .await?;

        Ok(Self::ndjson_stream(response.bytes_stream()))
    }

    // Buffered NDJSON decoder. Network chunks may split lines or contain many lines.
    fn ndjson_stream(
        bytes_stream: impl Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
    ) -> BackendStream {
        Box::pin(async_stream::stream! {
            let mut buffer = String::new();
            futures_util::pin_mut!(bytes_stream);

            while let Some(chunk_result) = bytes_stream.next().await {
                let chunk = match chunk_result {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        yield Err(error.to_string());
                        return;
                    }
                };

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim().to_string();
                    buffer.drain(..=pos);

                    if line.is_empty() {
                        continue;
                    }

                    match serde_json::from_str::<Value>(&line) {
                        Ok(value) => yield Ok(value),
                        Err(error) => {
                            yield Err(format!("INVALID_NDJSON: {}", error));
                            return;
                        }
                    }
                }
            }

            let tail = buffer.trim();
            if !tail.is_empty() {
                match serde_json::from_str::<Value>(tail) {
                    Ok(value) => yield Ok(value),
                    Err(error) => yield Err(format!("INVALID_NDJSON: {}", error)),
                }
            }
        })
    }
}
