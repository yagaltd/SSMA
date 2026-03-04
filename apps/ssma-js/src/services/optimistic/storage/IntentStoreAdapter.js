import { randomUUID } from "node:crypto";

const DEFAULT_REPLAY_WINDOW = 5 * 60 * 1000;
const CRDT_TYPES = new Set(["lww-register", "g-counter", "pn-counter"]);

export class IntentStoreAdapter {
  constructor(options = {}) {
    this.replayWindowMs = options.replayWindowMs || DEFAULT_REPLAY_WINDOW;
    this.maxEntries = options.maxEntries || 5000;
  }

  append() {
    throw new Error("append not implemented for IntentStoreAdapter");
  }

  entries() {
    throw new Error("entries not implemented for IntentStoreAdapter");
  }

  entriesSince() {
    throw new Error("entriesSince not implemented for IntentStoreAdapter");
  }

  get() {
    throw new Error("get not implemented for IntentStoreAdapter");
  }

  releaseReason() {
    throw new Error("releaseReason not implemented for IntentStoreAdapter");
  }

  addReason() {
    throw new Error("addReason not implemented for IntentStoreAdapter");
  }

  updateStatus() {
    throw new Error("updateStatus not implemented for IntentStoreAdapter");
  }

  entriesAfter(cursor = 0, { limit = 200, channels } = {}) {
    const all = this.entries();
    const filtered = all.filter((entry) => {
      const seq = entry.logSeq || entry.insertedAt;
      if (seq <= cursor) return false;
      if (channels && channels.length) {
        const entryChannels = Array.isArray(entry.meta?.channels)
          ? entry.meta.channels
          : ["global"];
        return channels.some((channel) => entryChannels.includes(channel));
      }
      return true;
    });
    return filtered.slice(0, limit);
  }

  latestCursor() {
    const entries = this.entries();
    if (!entries.length) return 0;
    const last = entries[entries.length - 1];
    return last.logSeq || last.insertedAt || 0;
  }

  count() {
    return this.entries().length;
  }

  normalizeMeta(meta = {}) {
    const reasons = Array.isArray(meta.reasons)
      ? Array.from(
          new Set(meta.reasons.map((reason) => String(reason).slice(0, 64))),
        ).slice(0, 16)
      : [];

    const normalized = {
      clock: Number.isFinite(meta.clock) ? meta.clock : Date.now(),
      reasons,
      channels: this.normalizeChannels(meta.channels),
    };

    for (const [key, value] of Object.entries(meta)) {
      if (
        [
          "reasons",
          "clock",
          "channels",
          "reducer",
          "actionCreator",
          "actor",
          "crdt",
        ].includes(key)
      )
        continue;
      normalized[key] = value;
    }

    if (meta.reducer) {
      normalized.reducer = String(meta.reducer).slice(0, 160);
    }
    if (meta.actionCreator) {
      normalized.actionCreator = String(meta.actionCreator).slice(0, 160);
    }
    if (meta.actor) {
      normalized.actor = String(meta.actor).slice(0, 96);
    }

    if (meta.crdt) {
      const descriptor = this._sanitizeCrdt(meta.crdt, normalized);
      if (descriptor) {
        normalized.crdt = descriptor;
      }
    }

    return normalized;
  }

  normalizeChannels(channels) {
    if (Array.isArray(channels) && channels.length) {
      return Array.from(
        new Set(channels.map((channel) => String(channel).slice(0, 64))),
      ).slice(0, 8);
    }
    return ["global"];
  }

  createEntry(intent = {}, context = {}) {
    const meta = this.normalizeMeta(intent.meta);
    if (!meta.actor && context.connectionId) {
      meta.actor = context.connectionId;
    }
    if (!meta.reasons.includes("pending")) meta.reasons.push("pending");
    if (!meta.reasons.includes("replay")) meta.reasons.push("replay");
    for (const channel of meta.channels) {
      const reason = `channel:${channel}`;
      if (!meta.reasons.includes(reason)) {
        meta.reasons.push(reason);
      }
    }

    const now = Date.now();
    return {
      id: intent.id || randomUUID(),
      intent: intent.intent,
      payload: intent.payload ?? null,
      meta,
      site: context.site || "default",
      status: "acked",
      connectionId: context.connectionId,
      insertedAt: now,
      updatedAt: now,
    };
  }

  _sanitizeCrdt(raw = {}, meta = {}) {
    if (typeof raw !== "object" || raw === null) return undefined;
    const type = String(raw.type || "").toLowerCase();
    if (!CRDT_TYPES.has(type)) return undefined;
    const key = raw.key ? String(raw.key).slice(0, 160) : null;
    if (!key) return undefined;
    const descriptor = { type, key };
    const reducer = raw.reducer || meta.reducer;
    if (reducer) {
      descriptor.reducer = String(reducer).slice(0, 160);
    }
    const actor = raw.actor || meta.actor;
    if (actor) {
      descriptor.actor = String(actor).slice(0, 96);
    }
    const timestamp = Number.isFinite(raw.timestamp)
      ? Number(raw.timestamp)
      : Number(meta.clock);
    if (Number.isFinite(timestamp)) {
      descriptor.timestamp = timestamp;
    }
    if (type === "lww-register") {
      descriptor.value = raw.value ?? null;
      if (raw.field) {
        descriptor.field = String(raw.field).slice(0, 80);
      }
    } else {
      const delta = Number(raw.delta ?? 0);
      if (Number.isFinite(delta)) {
        descriptor.delta = delta;
      }
    }
    if (raw.metadata && typeof raw.metadata === "object") {
      descriptor.metadata = raw.metadata;
    }
    return descriptor;
  }
}
