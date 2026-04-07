import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import path from 'node:path';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import { IntentStore } from '../src/services/optimistic/IntentStore.js';

describe('IntentStore', () => {
  let filePath;
  let store;

  beforeEach(() => {
    filePath = path.join(tmpdir(), `intent-store-${Date.now()}-${Math.random()}.json`);
    store = new IntentStore({ filePath, replayWindowMs: 50, maxEntries: 100 });
  });

  afterEach(() => {
    fs.rmSync(filePath, { force: true });
  });

  it('adds pending and replay reasons and removes pending on release without deleting entry', () => {
    const [entry] = store.append([
      { id: 'reason-test', intent: 'DEMO', payload: { foo: 'bar' }, meta: { clock: 1 } }
    ]);

    expect(entry.meta.reasons).toContain('pending');
    expect(entry.meta.reasons).toContain('replay');

    store.releaseReason(entry.id, 'pending');
    const persisted = store.get(entry.id);
    expect(persisted.meta.reasons).not.toContain('pending');
    expect(persisted.meta.reasons).toContain('replay');
    expect(persisted).toBeTruthy();
  });

  it('adds custom reasons without duplication', () => {
    store.append([{ id: 'reason-add', intent: 'DEMO', payload: {}, meta: { clock: 3 } }]);
    const added = store.addReason('reason-add', 'rework');
    expect(added).toBe(true);
    const again = store.addReason('reason-add', 'rework');
    expect(again).toBe(false);
  });

  it('sweeps replay reasons after expiry window but keeps entry for history', async () => {
    store.append([{ id: 'expire-test', intent: 'DEMO', payload: {}, meta: { clock: 2 } }]);
    store.releaseReason('expire-test', 'pending');
    store.releaseReason('expire-test', 'channel:global');

    await new Promise((resolve) => setTimeout(resolve, 60));
    const entries = store.entries();
    const target = entries.find((entry) => entry.id === 'expire-test');
    expect(target).toBeDefined();
    expect(target.meta.reasons).not.toContain('replay');
  });

  it('supports cursor-based entry retrieval', () => {
    const first = store.append([{ id: 'cursor-1', intent: 'ONE', payload: {} }])[0];
    const second = store.append([{ id: 'cursor-2', intent: 'TWO', payload: {} }])[0];
    const afterFirst = store.entriesAfter(first.logSeq || 0, { limit: 10 });
    expect(afterFirst.some((entry) => entry.id === 'cursor-2')).toBe(true);
    expect(afterFirst.some((entry) => entry.id === 'cursor-1')).toBe(false);
    expect(store.latestCursor()).toBe(second.logSeq || second.insertedAt);
  });

  it('normalizes crdt metadata on append', () => {
    const [entry] = store.append([
      {
        id: 'crdt-test',
        intent: 'SET',
        payload: { id: 'sku-1', qty: 5 },
        meta: {
          reducer: 'inventory.quantity',
          actionCreator: 'inventory.set',
          crdt: { type: 'lww-register', key: 'inventory:sku-1', value: { qty: 5 } }
        }
      }
    ]);

    expect(entry.meta.reducer).toBe('inventory.quantity');
    expect(entry.meta.actionCreator).toBe('inventory.set');
    expect(entry.meta.crdt).toEqual(
      expect.objectContaining({ type: 'lww-register', key: 'inventory:sku-1' })
    );
  });
});
