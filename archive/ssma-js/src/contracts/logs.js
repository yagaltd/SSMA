export const logsContracts = {
  "INTENT_LOG_BATCH": {
    "version": 1,
    "type": "intent",
    "owner": "log-service",
    "schema": {
      "$defs": {
        "logEntry": {
          "type": "object",
          "properties": {
            "event": { "type": "string", "minLength": 1, "maxLength": 64 },
            "level": { "type": "string", "enum": ["debug", "info", "warn", "error"] },
            "message": { "type": "string", "minLength": 1, "maxLength": 512 },
            "tags": {
              "type": "array",
              "maxItems": 16,
              "items": { "type": "string", "minLength": 1, "maxLength": 48 }
            },
            "context": {
              "type": "object",
              "additionalProperties": true
            },
            "timestamp": { "type": "number", "minimum": 0 },
            "durationMs": { "type": "number", "minimum": 0 }
          },
          "required": ["event", "level", "timestamp"],
          "additionalProperties": true
        },
        "integrity": {
          "type": "object",
          "properties": {
            "intent": { "type": "string", "minLength": 1 },
            "payloadHash": { "type": "string", "minLength": 32 },
            "nonce": { "type": "string", "minLength": 16 },
            "timestamp": { "type": "number", "minimum": 0 },
            "signature": { "type": "string", "minLength": 32 }
          },
          "required": ["intent", "payloadHash", "nonce", "timestamp", "signature"],
          "additionalProperties": false
        }
      },
      "type": "object",
      "properties": {
        "batchId": { "type": "string", "minLength": 10, "maxLength": 64 },
        "sessionId": { "type": "string", "minLength": 5 },
        "userId": { "type": "string", "minLength": 5 },
        "source": { "type": "string", "minLength": 1, "maxLength": 64 },
        "meta": {
          "type": "object",
          "properties": {
            "clientTime": { "type": "number", "minimum": 0 },
            "appVersion": { "type": "string", "maxLength": 32 },
            "platform": { "type": "string", "maxLength": 32 },
            "locale": { "type": "string", "maxLength": 16 }
          },
          "additionalProperties": true
        },
        "entries": {
          "type": "array",
          "minItems": 1,
          "maxItems": 200,
          "items": { "$ref": "#/$defs/logEntry" }
        },
        "integrity": { "$ref": "#/$defs/integrity" }
      },
      "required": ["batchId", "entries"],
      "additionalProperties": false
    }
  },
  "LOG_EVENT_RECEIVED": {
    "version": 1,
    "type": "event",
    "owner": "log-service",
    "schema": {
      "type": "object",
      "properties": {
        "batchId": { "type": "string" },
        "ackId": { "type": "string" },
        "accepted": { "type": "integer", "minimum": 0 },
        "rejected": { "type": "integer", "minimum": 0 },
        "receivedAt": { "type": "number", "minimum": 0 },
        "source": { "type": "string" },
        "bufferSize": { "type": "integer", "minimum": 0 },
        "bufferLimit": { "type": "integer", "minimum": 0 }
      },
      "required": ["batchId", "ackId", "accepted", "rejected", "receivedAt"],
      "additionalProperties": false
    }
  },
  "LOG_EVENT_PERSISTED": {
    "version": 1,
    "type": "event",
    "owner": "log-service",
    "schema": {
      "type": "object",
      "properties": {
        "batchId": { "type": "string" },
        "ackId": { "type": "string" },
        "bufferSize": { "type": "integer", "minimum": 0 },
        "bufferLimit": { "type": "integer", "minimum": 0 },
        "storedEntries": { "type": "integer", "minimum": 0 },
        "persistedAt": { "type": "number", "minimum": 0 }
      },
      "required": ["batchId", "ackId", "bufferSize", "bufferLimit", "persistedAt"],
      "additionalProperties": false
    }
  }
}
;
export default logsContracts;
