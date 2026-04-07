export const optimisticContracts = {
  "INTENT_BATCH": {
    "version": 1,
    "type": "intent",
    "owner": "optimistic-sync",
    "schema": {
      "$defs": {
        "intentEntry": {
          "type": "object",
          "properties": {
            "id": { "type": "string", "minLength": 8, "maxLength": 64 },
            "intent": { "type": "string", "minLength": 1, "maxLength": 96 },
            "payload": {
              "type": [
                "object",
                "array",
                "string",
                "number",
                "boolean",
                "null"
              ]
            },
            "meta": {
              "type": "object",
              "properties": {
                "clock": { "type": "number", "minimum": 0 },
                "reasons": {
                  "type": "array",
                  "maxItems": 16,
                  "items": { "type": "string", "minLength": 1, "maxLength": 64 }
                },
                "clientId": { "type": "string", "maxLength": 64 },
                "site": { "type": "string", "maxLength": 64 }
              },
              "required": ["clock"],
              "additionalProperties": true
            }
          },
          "required": ["intent", "payload", "meta"],
          "additionalProperties": true
        }
      },
      "type": "object",
      "properties": {
        "type": { "const": "intent.batch" },
        "intents": {
          "type": "array",
          "minItems": 1,
          "maxItems": 50,
          "items": { "$ref": "#/$defs/intentEntry" }
        }
      },
      "required": ["type", "intents"],
      "additionalProperties": false
    }
  },
  "EVENT_INVALIDATION": {
    "version": 1,
    "type": "event",
    "owner": "optimistic-sync",
    "schema": {
      "$defs": {
        "intentEntry": {
          "type": "object",
          "properties": {
            "id": { "type": "string" },
            "intent": { "type": "string" },
            "payload": {
              "type": [
                "object",
                "array",
                "string",
                "number",
                "boolean",
                "null"
              ]
            },
            "meta": { "type": "object", "additionalProperties": true },
            "insertedAt": { "type": "number", "minimum": 0 }
          },
          "required": ["id", "intent"],
          "additionalProperties": true
        }
      },
      "type": "object",
      "properties": {
        "reason": { "type": "string", "maxLength": 64 },
        "site": { "type": "string", "maxLength": 64 },
        "intents": {
          "type": "array",
          "items": { "$ref": "#/$defs/intentEntry" }
        }
      },
      "required": ["intents"],
      "additionalProperties": false
    }
  },
  "PING": {
    "version": 1,
    "type": "intent",
    "owner": "optimistic-sync",
    "schema": {
      "type": "object",
      "properties": {
        "type": { "const": "ping" }
      },
      "required": ["type"],
      "additionalProperties": false
    }
  }
}
;
export default optimisticContracts;
