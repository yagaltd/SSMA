use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendUser {
    pub id: Option<String>,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendContext {
    pub site: String,
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
        self.post_json("/apply-intents", serde_json::json!({
            "intents": intents,
            "context": context
        }))
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
        self.post_json("/health", serde_json::json!({ "context": context })).await
    }

    async fn post_json(&self, path: &str, payload: Value) -> Result<Value, reqwest::Error> {
        let url = format!("{}{}", self.base_url, path);
        let response = self.client.post(url).json(&payload).send().await?;
        response.json::<Value>().await
    }
}
