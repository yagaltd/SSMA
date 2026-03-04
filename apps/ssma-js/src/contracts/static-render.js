export const staticRenderContracts = {
  "ISLAND_HYDRATION_INITIATED": {
    "version": 1,
    "type": "event",
    "owner": "static-render",
    "schema": {
      "$defs": {
        "parameters": {
          "type": "object",
          "additionalProperties": true
        }
      },
      "type": "object",
      "properties": {
        "islandId": { "type": "string", "minLength": 1 },
        "route": { "type": "string", "minLength": 1 },
        "trigger": { "type": "string", "enum": ["load", "visible", "idle", "manual"] },
        "parameters": { "$ref": "#/$defs/parameters" },
        "timestamp": { "type": "number", "minimum": 0 }
      },
      "required": ["islandId", "route", "trigger", "timestamp"],
      "additionalProperties": false
    }
  },
  "ISLAND_DATA_REQUESTED": {
    "version": 1,
    "type": "event",
    "owner": "static-render",
    "schema": {
      "$defs": {
        "parameters": {
          "type": "object",
          "additionalProperties": true
        }
      },
      "type": "object",
      "properties": {
        "islandId": { "type": "string", "minLength": 1 },
        "dataContract": { "type": "string" },
        "parameters": { "$ref": "#/$defs/parameters" },
        "requestId": { "type": "string" },
        "timestamp": { "type": "number", "minimum": 0 }
      },
      "required": ["islandId", "timestamp"],
      "additionalProperties": false
    }
  },
  "ISLAND_INVALIDATED": {
    "version": 1,
    "type": "event",
    "owner": "static-render",
    "schema": {
      "$defs": {
        "parameters": {
          "type": "object",
          "additionalProperties": true
        }
      },
      "type": "object",
      "properties": {
        "islandId": { "type": "string", "minLength": 1 },
        "parameters": { "$ref": "#/$defs/parameters" },
        "reason": { "type": "string" },
        "site": { "type": "string" },
        "cursor": { "type": "number", "minimum": 0 },
        "timestamp": { "type": "number", "minimum": 0 },
        "dataContract": { "type": "string" },
        "payload": { "type": "object", "additionalProperties": true },
        "eventId": { "type": "string" },
        "type": { "type": "string" }
      },
      "required": ["islandId", "timestamp"],
      "additionalProperties": false
    }
  },
  "ISLAND_DATA_RETURNED": {
    "version": 1,
    "type": "event",
    "owner": "static-render",
    "schema": {
      "$defs": {
        "payload": {
          "type": "object",
          "additionalProperties": true
        },
        "parameters": {
          "type": "object",
          "additionalProperties": true
        }
      },
      "type": "object",
      "properties": {
        "islandId": { "type": "string", "minLength": 1 },
        "dataContract": { "type": "string", "minLength": 1 },
        "parameters": { "$ref": "#/$defs/parameters" },
        "payload": { "$ref": "#/$defs/payload" },
        "timestamp": { "type": "number", "minimum": 0 },
        "requestId": { "type": "string" }
      },
      "required": ["islandId", "dataContract", "timestamp"],
      "additionalProperties": false
    }
  },
  "ISLAND_HYDRATION_FAILED": {
    "version": 1,
    "type": "event",
    "owner": "static-render",
    "schema": {
      "$defs": {
        "parameters": {
          "type": "object",
          "additionalProperties": true
        }
      },
      "type": "object",
      "properties": {
        "islandId": { "type": "string", "minLength": 1 },
        "reason": { "type": "string" },
        "error": { "type": "string" },
        "parameters": { "$ref": "#/$defs/parameters" },
        "timestamp": { "type": "number", "minimum": 0 }
      },
      "required": ["islandId", "timestamp"],
      "additionalProperties": false
    }
  }
}
;
export default staticRenderContracts;
