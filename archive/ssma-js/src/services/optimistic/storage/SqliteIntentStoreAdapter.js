import Database from "better-sqlite3";
import path from "node:path";
import fs from "node:fs";
import { IntentStoreAdapter } from "./IntentStoreAdapter.js";

export class SqliteIntentStoreAdapter extends IntentStoreAdapter {
  constructor(options = {}) {
    super(options);
    const defaultPath = path.resolve(
      process.cwd(),
      "data/optimistic-intents.sqlite",
    );
    this.dbPath = options.sqlitePath || options.storePath || defaultPath;
    const dir = path.dirname(this.dbPath);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true });
    }
    this.db = new Database(this.dbPath);
    this.db.pragma("journal_mode = WAL");
    this.db.pragma("synchronous = NORMAL");
    this._prepare();
    this.sequence = this._loadSequence();
    this._ensureLogSeq();
    this.insertStmt = this.db.prepare(`
      INSERT INTO intents (id, intent, payload, meta, site, status, connectionId, insertedAt, updatedAt, logSeq)
      VALUES (@id, @intent, @payload, @meta, @site, @status, @connectionId, @insertedAt, @updatedAt, @logSeq)
      ON CONFLICT(id) DO UPDATE SET
        intent=excluded.intent,
        payload=excluded.payload,
        meta=excluded.meta,
        site=excluded.site,
        status=excluded.status,
        connectionId=excluded.connectionId,
        insertedAt=excluded.insertedAt,
        updatedAt=excluded.updatedAt,
        logSeq=excluded.logSeq
    `);
    this.selectAllStmt = this.db.prepare(
      "SELECT * FROM intents ORDER BY logSeq ASC",
    );
    this.selectSinceStmt = this.db.prepare(
      "SELECT * FROM intents WHERE insertedAt >= ? ORDER BY logSeq ASC",
    );
    this.selectByIdStmt = this.db.prepare("SELECT * FROM intents WHERE id = ?");
    this.updateMetaStmt = this.db.prepare(
      "UPDATE intents SET meta = @meta, updatedAt = @updatedAt WHERE id = @id",
    );
    this.updateStatusStmt = this.db.prepare(
      "UPDATE intents SET status = @status, meta = @meta, updatedAt = @updatedAt WHERE id = @id",
    );
    this.entriesAfterStmt = this.db.prepare(
      "SELECT * FROM intents WHERE logSeq > ? ORDER BY logSeq ASC LIMIT ?",
    );
  }

  append(intents = [], context = {}) {
    if (!Array.isArray(intents) || intents.length === 0) return [];
    const store = this;
    const now = Date.now();
    const insertMany = this.db.transaction((entries) => {
      for (const entry of entries) {
        store.insertStmt.run({
          ...entry,
          payload: JSON.stringify(entry.payload ?? null),
          meta: JSON.stringify(entry.meta ?? {}),
          insertedAt: entry.insertedAt || now,
          updatedAt: entry.updatedAt || now,
          logSeq: entry.logSeq,
        });
      }
    });

    const entries = intents
      .filter((intent) => intent && typeof intent === "object")
      .map((intent) => this.createEntry(intent, context));
    for (const entry of entries) {
      if (!Number.isFinite(entry.logSeq) || entry.logSeq <= 0) {
        entry.logSeq = this._nextSeq();
      }
    }
    insertMany(entries);
    return entries;
  }

  entries() {
    return this.selectAllStmt.all().map((row) => this._hydrate(row));
  }

  entriesSince(timestamp = 0) {
    return this.selectSinceStmt.all(timestamp).map((row) => this._hydrate(row));
  }

  get(id) {
    const row = this.selectByIdStmt.get(id);
    return row ? this._hydrate(row) : null;
  }

  releaseReason(id, reason) {
    const entry = this.get(id);
    if (!entry) return false;
    const before = entry.meta.reasons.length;
    entry.meta.reasons = entry.meta.reasons.filter((item) => item !== reason);
    if (entry.meta.reasons.length !== before) {
      this._saveMeta(entry);
      return true;
    }
    return false;
  }

  addReason(id, reason) {
    const entry = this.get(id);
    if (!entry) return false;
    entry.meta.reasons = entry.meta.reasons || [];
    if (!entry.meta.reasons.includes(reason)) {
      entry.meta.reasons.push(reason);
      this._saveMeta(entry);
      return true;
    }
    return false;
  }

  updateStatus(id, status, meta = {}) {
    const entry = this.get(id);
    if (!entry) return false;
    entry.status = status || entry.status;
    entry.meta = {
      ...(entry.meta || {}),
      ...(meta || {}),
    };
    this.updateStatusStmt.run({
      id: entry.id,
      status: entry.status,
      meta: JSON.stringify(entry.meta),
      updatedAt: Date.now(),
    });
    return true;
  }

  _saveMeta(entry) {
    this.updateMetaStmt.run({
      id: entry.id,
      meta: JSON.stringify(entry.meta),
      updatedAt: Date.now(),
    });
  }

  _hydrate(row) {
    let payload;
    let meta;
    try {
      payload = row.payload ? JSON.parse(row.payload) : null;
    } catch {
      payload = null;
    }
    try {
      meta = row.meta ? JSON.parse(row.meta) : {};
    } catch {
      meta = {};
    }
    meta.reasons = Array.isArray(meta.reasons) ? meta.reasons : [];
    meta.channels =
      Array.isArray(meta.channels) && meta.channels.length
        ? meta.channels
        : ["global"];
    return {
      id: row.id,
      intent: row.intent,
      payload,
      meta,
      site: row.site,
      status: row.status,
      connectionId: row.connectionId,
      insertedAt: row.insertedAt,
      updatedAt: row.updatedAt,
      logSeq: row.logSeq || 0,
    };
  }

  _prepare() {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS intents (
        id TEXT PRIMARY KEY,
        intent TEXT NOT NULL,
        payload TEXT,
        meta TEXT,
        site TEXT,
        status TEXT,
        connectionId TEXT,
        insertedAt INTEGER,
        updatedAt INTEGER,
        logSeq INTEGER
      );
      CREATE INDEX IF NOT EXISTS idx_intents_inserted_at ON intents(insertedAt);
      CREATE INDEX IF NOT EXISTS idx_intents_site ON intents(site);
      CREATE INDEX IF NOT EXISTS idx_intents_logseq ON intents(logSeq);
      CREATE TABLE IF NOT EXISTS intent_metadata (
        key TEXT PRIMARY KEY,
        value INTEGER
      );
    `);
    this._ensureLogSeqColumn();
  }

  entriesAfter(cursor = 0, { limit = 200, channels } = {}) {
    const rows = this.entriesAfterStmt.all(cursor, limit * 2);
    const hydrated = rows.map((row) => this._hydrate(row));
    const filtered = hydrated.filter((entry) => {
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
    const row = this.db
      .prepare("SELECT MAX(logSeq) as maxSeq FROM intents")
      .get();
    return row?.maxSeq || 0;
  }

  _ensureLogSeqColumn() {
    const columns = this.db.prepare("PRAGMA table_info(intents)").all();
    const hasLogSeq = columns.some((column) => column.name === "logSeq");
    if (!hasLogSeq) {
      this.db.exec("ALTER TABLE intents ADD COLUMN logSeq INTEGER");
    }
  }

  _loadSequence() {
    const row = this.db
      .prepare("SELECT value FROM intent_metadata WHERE key = ?")
      .get("logSeq");
    if (row && Number.isFinite(row.value)) {
      return row.value;
    }
    const maxRow = this.db
      .prepare("SELECT MAX(logSeq) as maxSeq FROM intents")
      .get();
    const next = (maxRow?.maxSeq || 0) + 1;
    this.db
      .prepare(
        "INSERT INTO intent_metadata(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
      )
      .run("logSeq", next);
    return next;
  }

  _saveSequence(value) {
    this.db
      .prepare(
        "INSERT INTO intent_metadata(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
      )
      .run("logSeq", value);
  }

  _nextSeq() {
    const current = this.sequence;
    this.sequence += 1;
    this._saveSequence(this.sequence);
    return current;
  }

  _ensureLogSeq() {
    const missing = this.db
      .prepare(
        "SELECT id FROM intents WHERE logSeq IS NULL OR logSeq = 0 ORDER BY insertedAt ASC",
      )
      .all();
    if (!missing.length) {
      return;
    }
    const update = this.db.prepare(
      "UPDATE intents SET logSeq = @logSeq WHERE id = @id",
    );
    this.db.transaction(() => {
      for (const row of missing) {
        const seq = this._nextSeq();
        update.run({ id: row.id, logSeq: seq });
      }
    })();
  }

  count() {
    const row = this.db.prepare("SELECT COUNT(*) as count FROM intents").get();
    return row?.count || 0;
  }
}
