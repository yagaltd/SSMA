use anyhow::Result;
use serde_json::Value;

#[test]
fn vector_harness_loads_shared_vectors() -> Result<()> {
    let root = std::path::PathBuf::from("../../packages/ssma-protocol/vectors");
    assert!(root.exists(), "shared vectors directory must exist");

    for file in std::fs::read_dir(root)? {
        let file = file?;
        let path = file.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = std::fs::read_to_string(path)?;
        let parsed: Value = serde_json::from_str(&raw)?;
        assert!(parsed.get("name").is_some());
        assert!(parsed.get("client").and_then(|v| v.as_array()).is_some());
        assert!(parsed.get("server").and_then(|v| v.as_array()).is_some());
    }
    Ok(())
}

#[test]
fn strict_contract_validation_rejects_invalid_payload() {
    let invalid = serde_json::json!({
        "type": "channel.command",
        "channel": "global",
        "command": { "bad": true }
    });
    let result = ssma_rust::protocol::validate_inbound(&invalid);
    assert!(result.is_err());
}
