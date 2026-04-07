import { FileIntentStoreAdapter } from "./storage/FileIntentStoreAdapter.js";
import { SqliteIntentStoreAdapter } from "./storage/SqliteIntentStoreAdapter.js";

const adapters = new Map([
  ["file", FileIntentStoreAdapter],
  ["sqlite", SqliteIntentStoreAdapter],
]);

export function registerIntentStoreAdapter(type, AdapterClass) {
  if (!type || typeof AdapterClass !== "function") {
    throw new Error(
      "[IntentStore] registerIntentStoreAdapter requires a type and class",
    );
  }
  adapters.set(type, AdapterClass);
}

export class IntentStore {
  constructor(options = {}) {
    const requested = options.adapter?.type || "file";
    const AdapterClass = adapters.get(requested) || FileIntentStoreAdapter;
    this.monitor = options.monitor || null;
    if (options.adapter?.instance) {
      this.adapter = options.adapter.instance;
    } else {
      this.adapter = new AdapterClass(options);
    }
    this.#emitBacklog();
  }

  append(intents, context) {
    const stored = this.adapter.append(intents, context);
    this.#emitBacklog();
    return stored;
  }

  entries() {
    return this.adapter.entries();
  }

  entriesSince(timestamp) {
    return this.adapter.entriesSince(timestamp);
  }

  entriesAfter(cursor, options) {
    if (typeof this.adapter.entriesAfter === "function") {
      return this.adapter.entriesAfter(cursor, options);
    }
    return this.adapter
      .entries()
      .filter((entry) => (entry.logSeq || entry.insertedAt || 0) > cursor);
  }

  latestCursor() {
    if (typeof this.adapter.latestCursor === "function") {
      return this.adapter.latestCursor();
    }
    const entries = this.adapter.entries();
    if (!entries.length) return 0;
    const last = entries[entries.length - 1];
    return last.logSeq || last.insertedAt || 0;
  }

  get(id) {
    return this.adapter.get(id);
  }

  releaseReason(id, reason) {
    return this.adapter.releaseReason(id, reason);
  }

  addReason(id, reason) {
    return this.adapter.addReason?.(id, reason);
  }

  updateStatus(id, status, meta = {}) {
    return this.adapter.updateStatus?.(id, status, meta);
  }

  #emitBacklog() {
    if (!this.monitor) return;
    const count =
      typeof this.adapter.count === "function"
        ? this.adapter.count()
        : this.adapter.entries().length;
    this.monitor.recordBacklog(count);
  }
}
