import fs from "node:fs";
import path from "node:path";
import { IntentStoreAdapter } from "./IntentStoreAdapter.js";

export class FileIntentStoreAdapter extends IntentStoreAdapter {
  constructor(options = {}) {
    super(options);
    this.filePath =
      options.filePath ||
      path.resolve(process.cwd(), "data/optimistic-intents.json");
    this.store = new Map();
    this.nextSeq = 1;
    this._loadFromDisk();
  }

  append(intents = [], context = {}) {
    const stored = [];
    this._sweepExpired();

    for (const intent of intents) {
      if (!intent || typeof intent !== "object") continue;
      const entry = this.createEntry(intent, context);
      entry.logSeq = entry.logSeq || this._nextSeq();
      this.store.set(entry.id, entry);
      stored.push(entry);
    }

    if (stored.length > 0) {
      this._trim();
      this._persist();
    }

    return stored;
  }

  entries() {
    this._sweepExpired();
    return Array.from(this.store.values()).sort(
      (a, b) => a.insertedAt - b.insertedAt,
    );
  }

  entriesSince(timestamp = 0) {
    return this.entries().filter((entry) => entry.insertedAt >= timestamp);
  }

  get(id) {
    this._sweepExpired();
    return this.store.get(id) || null;
  }

  releaseReason(id, reason) {
    const entry = this.store.get(id);
    if (!entry) return false;
    const before = entry.meta.reasons.length;
    entry.meta.reasons = entry.meta.reasons.filter((item) => item !== reason);
    if (entry.meta.reasons.length !== before) {
      this._persist();
      return true;
    }
    return false;
  }

  addReason(id, reason) {
    const entry = this.store.get(id);
    if (!entry) return false;
    entry.meta.reasons = entry.meta.reasons || [];
    if (!entry.meta.reasons.includes(reason)) {
      entry.meta.reasons.push(reason);
      this._persist();
      return true;
    }
    return false;
  }

  updateStatus(id, status, meta = {}) {
    const entry = this.store.get(id);
    if (!entry) return false;
    entry.status = status || entry.status;
    entry.updatedAt = Date.now();
    if (meta && typeof meta === "object") {
      entry.backend = {
        ...(entry.backend || {}),
        ...meta,
      };
    }
    this._persist();
    return true;
  }

  _loadFromDisk() {
    try {
      const dir = path.dirname(this.filePath);
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }
      if (!fs.existsSync(this.filePath)) {
        fs.writeFileSync(
          this.filePath,
          JSON.stringify({ entries: [], nextSeq: 1 }, null, 2),
          "utf8",
        );
        return;
      }
      const raw = fs.readFileSync(this.filePath, "utf8");
      if (!raw) return;
      const parsed = JSON.parse(raw);
      const entries = Array.isArray(parsed)
        ? parsed
        : Array.isArray(parsed.entries)
          ? parsed.entries
          : [];
      this.nextSeq = Number(parsed.nextSeq) || 1;
      let maxSeq = 0;
      for (const entry of entries) {
        if (!entry?.id) continue;
        if (!Array.isArray(entry.meta?.reasons))
          entry.meta = { ...(entry.meta || {}), reasons: [] };
        if (
          !Array.isArray(entry.meta?.channels) ||
          entry.meta.channels.length === 0
        ) {
          entry.meta = { ...(entry.meta || {}), channels: ["global"] };
        }
        if (!Number.isFinite(entry.logSeq)) {
          entry.logSeq = ++maxSeq;
        } else if (entry.logSeq > maxSeq) {
          maxSeq = entry.logSeq;
        }
        this.store.set(entry.id, entry);
      }
      if (maxSeq + 1 > this.nextSeq) {
        this.nextSeq = maxSeq + 1;
      }
    } catch (error) {
      console.error("[IntentStore] Failed to load persisted intents:", error);
    }
  }

  _persist() {
    try {
      const payload = JSON.stringify(
        {
          nextSeq: this.nextSeq,
          entries: Array.from(this.store.values()).sort(
            (a, b) => a.logSeq - b.logSeq,
          ),
        },
        null,
        2,
      );
      fs.writeFileSync(this.filePath, payload, "utf8");
    } catch (error) {
      console.error("[IntentStore] Failed to persist intents:", error);
    }
  }

  _trim() {
    if (this.store.size <= this.maxEntries) return;
    const entries = Array.from(this.store.values()).sort(
      (a, b) => a.logSeq - b.logSeq,
    );
    const surplus = entries.length - this.maxEntries;
    for (let i = 0; i < surplus; i++) {
      this.store.delete(entries[i].id);
    }
  }

  _sweepExpired() {
    const now = Date.now();
    let changed = false;
    for (const entry of this.store.values()) {
      if (
        entry.meta.reasons.includes("replay") &&
        now - entry.insertedAt > this.replayWindowMs
      ) {
        entry.meta.reasons = entry.meta.reasons.filter(
          (reason) => reason !== "replay",
        );
        changed = true;
      }
    }
    if (changed) {
      this._persist();
    }
  }

  entriesAfter(cursor = 0, { limit = 200, channels } = {}) {
    const sorted = Array.from(this.store.values()).sort(
      (a, b) => a.logSeq - b.logSeq,
    );
    const filtered = sorted.filter((entry) => {
      if ((entry.logSeq || 0) <= cursor) return false;
      if (channels && channels.length) {
        const channelList = Array.isArray(entry.meta?.channels)
          ? entry.meta.channels
          : ["global"];
        return channels.some((channel) => channelList.includes(channel));
      }
      return true;
    });
    return filtered.slice(0, limit);
  }

  latestCursor() {
    const entries = Array.from(this.store.values()).sort(
      (a, b) => a.logSeq - b.logSeq,
    );
    if (!entries.length) return 0;
    return entries[entries.length - 1].logSeq || 0;
  }

  _nextSeq() {
    return this.nextSeq++;
  }

  count() {
    this._sweepExpired();
    return this.store.size;
  }
}
