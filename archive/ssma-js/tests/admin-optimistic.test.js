import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import http from 'node:http';
import path from 'node:path';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import { createNodeKernel } from '../src/runtime/kernel/nodeKernel.js';
import { registerOptimisticRoutes } from '../src/routes/optimisticRoutes.js';
import { ChannelRegistry } from '../src/services/optimistic/ChannelRegistry.js';
import { IntentStore } from '../src/services/optimistic/IntentStore.js';
import { OptimisticEventHub } from '../src/services/optimistic/OptimisticEventHub.js';

describe('Admin optimistic endpoints', () => {
  let store;
  let filePath;
  let server;
  let port;
  let channelRegistry;
  let currentUser;

  beforeEach(async () => {
    filePath = path.join(tmpdir(), `admin-intents-${Date.now()}.json`);
    store = new IntentStore({ filePath, maxEntries: 50 });
    store.append([
      {
        id: 'pending-1',
        intent: 'DEMO',
        payload: { foo: 'bar' },
        meta: { clock: 1, channels: ['global'], reasons: ['pending', 'channel:global'] }
      }
    ]);

    channelRegistry = new ChannelRegistry({ intentStore: store });
    channelRegistry.registerChannel('global', {
      load: ({ intentStore }) => intentStore.entries()
    });
    channelRegistry.attachConnection('conn-admin', {
      send: () => {},
      context: { role: 'follower', site: 'alpha', user: { id: 'client-1', role: 'user' } }
    });
    await channelRegistry.subscribe('conn-admin', { channel: 'global', params: {} });

    const kernel = createNodeKernel({ intentStore: store, optimistic: { replayWindowMs: 1000 } });
    currentUser = { id: 'admin', role: 'staff' };
    kernel.use((ctx, next) => {
      ctx.state.user = currentUser;
      return next();
    });

    registerOptimisticRoutes(kernel, new OptimisticEventHub(), channelRegistry, { ingestServerEvent: () => Promise.resolve() }, store);

    server = kernel.server;
    await new Promise((resolve) => server.listen(0, resolve));
    port = server.address().port;
  });

  afterEach(async () => {
    await new Promise((resolve) => server.close(resolve));
    if (fs.existsSync(filePath)) {
      fs.rmSync(filePath, { force: true });
    }
  });

  function requestJSON(pathname, { method = 'GET' } = {}) {
    return new Promise((resolve, reject) => {
      const req = http.request({
        hostname: '127.0.0.1',
        port,
        method,
        path: pathname,
        headers: { 'Content-Type': 'application/json' }
      }, (res) => {
        let body = '';
        res.on('data', (chunk) => { body += chunk; });
        res.on('end', () => {
          resolve({ status: res.statusCode, body: body ? JSON.parse(body) : {} });
        });
      });
      req.on('error', reject);
      req.end();
    });
  }

  it('returns channel subscription snapshots for staff users', async () => {
    const response = await requestJSON('/admin/optimistic/channels');
    expect(response.status).toBe(200);
    expect(response.body.totalSubscriptions).toBeGreaterThan(0);
    expect(Array.isArray(response.body.channels)).toBe(true);
    expect(response.body.channels[0].subscribers[0].site).toBe('alpha');
  });

  it('exposes pending intents with reason summaries', async () => {
    const response = await requestJSON('/admin/optimistic/intents');
    expect(response.status).toBe(200);
    expect(response.body.total).toBeGreaterThan(0);
    expect(response.body.reasonSummary.some((item) => item.reason === 'pending')).toBe(true);
  });

  it('enforces staff role for admin endpoints', async () => {
    currentUser = { id: 'user-1', role: 'user' };
    const response = await requestJSON('/admin/optimistic/channels');
    expect(response.status).toBe(403);
    expect(response.body.requiredRole).toBe('staff');
  });

  it('exposes transport metrics for monitoring', async () => {
    const response = await requestJSON('/optimistic/metrics');
    expect(response.status).toBe(200);
    expect(response.body).toHaveProperty('sse');
    expect(response.body).toHaveProperty('ws');
    expect(response.body.backlog).toBeGreaterThanOrEqual(0);
    expect(response.body.channelSubscriptions).toBeGreaterThanOrEqual(0);
  });
});
