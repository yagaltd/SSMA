export const channelsContracts = {
  "CHANNEL_SUBSCRIBE": {
    "version": 1,
    "type": "intent",
    "owner": "optimistic-sync",
    "schema": {
      "type": "object",
      "properties": {
        "type": { "const": "channel.subscribe" },
        "channel": { "type": "string", "minLength": 1, "maxLength": 96 },
        "params": { "type": "object", "additionalProperties": true },
        "filter": { "type": ["object", "null"], "additionalProperties": true }
      },
      "required": ["type", "channel"],
      "additionalProperties": false
    }
  },
  "CHANNEL_RESYNC": {
    "version": 1,
    "type": "intent",
    "owner": "optimistic-sync",
    "schema": {
      "type": "object",
      "properties": {
        "type": { "const": "channel.resync" },
        "channel": { "type": "string", "minLength": 1, "maxLength": 96 },
        "cursor": { "type": "number", "minimum": 0 },
        "limit": { "type": "number", "minimum": 1, "maximum": 1000 },
        "params": { "type": "object", "additionalProperties": true }
      },
      "required": ["type", "channel"],
      "additionalProperties": false
    }
  },
  "CHANNEL_UNSUBSCRIBE": {
    "version": 1,
    "type": "intent",
    "owner": "optimistic-sync",
    "schema": {
      "type": "object",
      "properties": {
        "type": { "const": "channel.unsubscribe" },
        "channel": { "type": "string", "minLength": 1, "maxLength": 96 },
        "params": { "type": "object", "additionalProperties": true }
      },
      "required": ["type", "channel"],
      "additionalProperties": false
    }
  },
  "CHANNEL_COMMAND": {
    "version": 1,
    "type": "intent",
    "owner": "optimistic-sync",
    "schema": {
      "type": "object",
      "properties": {
        "type": { "const": "channel.command" },
        "channel": { "type": "string", "minLength": 1, "maxLength": 96 },
        "params": { "type": "object", "additionalProperties": true },
        "command": { "type": "string", "minLength": 1, "maxLength": 128 },
        "args": { "type": "object", "additionalProperties": true }
      },
      "required": ["type", "channel", "command"],
      "additionalProperties": false
    }
  }
}
;
export default channelsContracts;
