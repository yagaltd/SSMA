import { describe, it, expect, beforeAll, afterAll, vi } from 'vitest';
import path from 'node:path';
import { tmpdir } from 'node:os';
import fs from 'node:fs';
import { createNodeKernel } from '../src/runtime/kernel/nodeKernel.js';
import { registerOptimisticRoutes } from '../src/routes/optimisticRoutes.js';
import { IntentStore } from '../src/services/optimistic/IntentStore.js';
import { OptimisticEventHub } from '../src/services/optimistic/OptimisticEventHub.js';
import { ChannelRegistry } from '../src/services/optimistic/ChannelRegistry.js';

describe('Optimistic rework routes', () => {
  let server;
  let port;
  let store;
  let filePath;
  let eventHub;
  let logService;

  beforeAll(async () => {
    filePath = path.join(tmpdir(), `intent-store-${Date.now()}-${Math.random()}.json`);
    store = new IntentStore({ filePath, maxEntries: 50 });
    eventHub = new OptimisticEventHub();
    eventHub.publish = vi.fn();

    const channelRegistry = new ChannelRegistry({ intentStore: store });

    const kernel = createNodeKernel({ intentStore: store, optimistic: { replayWindowMs: 1000 }, channelRegistry });
    kernel.use((ctx, next) => {
      ctx.state.user = { role: 'staff' };
      return next();
    });
    logService = { ingestServerEvent: vi.fn().mockResolvedValue() };
    registerOptimisticRoutes(kernel, eventHub, channelRegistry, logService, store);
    server = kernel.server;
    await new Promise((resolve) => server.listen(0, resolve));
    port = server.address().port;
  });

  afterAll(async () => {
    await new Promise((resolve) => server.close(resolve));
    fs.rmSync(filePath, { force: true });
  });

  it('publishes rework payload and accepts undo confirmation', async () => {
    const [entry] = store.append([
      {
        id: 'rework-test',
        intent: 'INTENT_TEST',
        payload: { foo: 'bar' },
        meta: {
          clock: 1,
          channels: ['global'],
          undo: { intent: 'INTENT_TEST_UNDO', payload: { foo: 'bar' } }
        }
      }
    ]);

    store.releaseReason(entry.id, 'pending');

    const reworkResponse = await fetch(`http://127.0.0.1:${port}/optimistic/rework`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: entry.id, reason: 'test-rework' })
    });

    const reworkBody = await reworkResponse.json();
    if (reworkResponse.status !== 202) {
      throw new Error(`Rework failed: ${JSON.stringify(reworkBody)}`);
    }
    expect(eventHub.publish).toHaveBeenCalledWith('rework', expect.any(Object));
    expect(logService.ingestServerEvent).toHaveBeenCalledWith(
      'OPTIMISTIC_REWORK_ENQUEUED',
      expect.objectContaining({ entryId: entry.id })
    );

    const undoResponse = await fetch(`http://127.0.0.1:${port}/optimistic/undo`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id: entry.id, intent: 'INTENT_TEST_UNDO', payload: { foo: 'bar' } })
    });

    const undoBody = await undoResponse.json();
    if (undoResponse.status !== 200) {
      throw new Error(`Undo failed: ${JSON.stringify(undoBody)}`);
    }
    expect(eventHub.publish).toHaveBeenCalledWith('undo', expect.any(Object));
    expect(logService.ingestServerEvent).toHaveBeenCalledWith(
      'OPTIMISTIC_UNDO_CONFIRMED',
      expect.objectContaining({ entryId: entry.id })
    );
    const persisted = store.get(entry.id);
    expect(persisted).toBeTruthy();
    expect(persisted.meta.reasons).not.toContain('replay');
    expect(persisted.meta.reasons).not.toContain('pending');
  });
});
