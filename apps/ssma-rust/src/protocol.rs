use jsonschema::JSONSchema;
use once_cell::sync::Lazy;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

pub fn contracts_dir() -> PathBuf {
    PathBuf::from("../../packages/ssma-protocol/contracts")
}

static VALIDATORS: Lazy<HashMap<String, JSONSchema>> = Lazy::new(|| {
    let mut out = HashMap::new();
    let files = ["optimistic.json", "channels.json", "errors.json"];
    for file in files {
        let full = contracts_dir().join(file);
        if !full.exists() {
            continue;
        }
        let Ok(raw) = fs::read_to_string(full) else {
            continue;
        };
        let Ok(parsed) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(group) = parsed.as_object() else {
            continue;
        };
        for (name, spec) in group {
            let Some(schema) = spec.get("schema") else {
                continue;
            };
            if let Ok(compiled) = JSONSchema::compile(schema) {
                out.insert(name.clone(), compiled);
            }
        }
    }
    out
});

pub fn contract_for_type(message_type: &str) -> Option<&'static str> {
    match message_type {
        "intent.batch" => Some("INTENT_BATCH"),
        "channel.subscribe" => Some("CHANNEL_SUBSCRIBE"),
        "channel.unsubscribe" => Some("CHANNEL_UNSUBSCRIBE"),
        "channel.resync" => Some("CHANNEL_RESYNC"),
        "channel.command" => Some("CHANNEL_COMMAND"),
        "ping" => Some("PING"),
        _ => None,
    }
}

pub fn validate_inbound(payload: &Value) -> Result<(), String> {
    let Some(message_type) = payload.get("type").and_then(|v| v.as_str()) else {
        return Err("missing message type".to_string());
    };
    let Some(contract) = contract_for_type(message_type) else {
        return Ok(());
    };
    let Some(validator) = VALIDATORS.get(contract) else {
        return Err(format!("missing validator for {}", contract));
    };
    match validator.validate(payload) {
        Ok(_) => Ok(()),
        Err(errors) => {
            let details = errors.map(|err| err.to_string()).collect::<Vec<_>>().join("; ");
            Err(details)
        }
    }
}
