import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import path from 'node:path';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import { SqliteIntentStoreAdapter } from '../src/services/optimistic/storage/SqliteIntentStoreAdapter.js';

describe('SqliteIntentStoreAdapter', () => {
  let dbFile;
  let adapter;

  beforeEach(() => {
    dbFile = path.join(tmpdir(), `intent-sqlite-${Date.now()}-${Math.random()}.sqlite`);
    adapter = new SqliteIntentStoreAdapter({ sqlitePath: dbFile, replayWindowMs: 50 });
  });

  afterEach(() => {
    if (fs.existsSync(dbFile)) {
      fs.rmSync(dbFile, { force: true });
    }
  });

  it('persists intents and retrieves them by id', () => {
    const [entry] = adapter.append([
      { id: 'sqlite-test', intent: 'DEMO', payload: { foo: 'bar' }, meta: { clock: 1 } }
    ]);
    expect(entry.id).toBe('sqlite-test');
    const stored = adapter.get('sqlite-test');
    expect(stored.payload).toEqual({ foo: 'bar' });
  });

  it('removes reasons without deleting entry so history remains', () => {
    adapter.append([
      { id: 'sqlite-reason', intent: 'DEMO', payload: {}, meta: { clock: 2 } }
    ]);
    adapter.releaseReason('sqlite-reason', 'pending');
    adapter.releaseReason('sqlite-reason', 'replay');
    adapter.releaseReason('sqlite-reason', 'channel:global');
    const stored = adapter.get('sqlite-reason');
    expect(stored).toBeTruthy();
    expect(stored.meta.reasons.length).toBe(0);
  });

  it('supports cursor queries via entriesAfter', () => {
    const first = adapter.append([{ id: 'sqlite-cursor-1', intent: 'ONE', payload: {} }])[0];
    const second = adapter.append([{ id: 'sqlite-cursor-2', intent: 'TWO', payload: {} }])[0];
    const entries = adapter.entriesAfter(first.logSeq || 0, { limit: 10 });
    expect(entries.some((entry) => entry.id === 'sqlite-cursor-2')).toBe(true);
    expect(entries.some((entry) => entry.id === 'sqlite-cursor-1')).toBe(false);
    expect(adapter.latestCursor()).toBe(second.logSeq);
  });
});
