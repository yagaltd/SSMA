import { describe, it, expect, beforeAll, afterAll, vi } from 'vitest';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { tmpdir } from 'node:os';
import { WebSocket } from 'ws';
import { IntentStore } from '../src/services/optimistic/IntentStore.js';
import { OptimisticEventHub } from '../src/services/optimistic/OptimisticEventHub.js';
import { SyncGateway } from '../src/services/optimistic/SyncGateway.js';
import { ChannelRegistry } from '../src/services/optimistic/ChannelRegistry.js';

describe('SyncGateway', () => {
  let server;
  let gateway;
  let port;
  let storeFile;
  let store;

  beforeAll(async () => {
    server = http.createServer((req, res) => {
      res.statusCode = 404;
      res.end('not-found');
    });

    storeFile = path.join(tmpdir(), `intent-store-${Date.now()}.json`);
    store = new IntentStore({ filePath: storeFile, maxEntries: 100 });

    const channelRegistry = new ChannelRegistry({ intentStore: store });
    channelRegistry.registerChannel('global', {
      load: ({ intentStore }) => intentStore.entriesSince(Date.now() - 1000)
    });

    gateway = new SyncGateway({
      server,
      intentStore: store,
      eventHub: new OptimisticEventHub(),
      channelRegistry,
      channelRateLimit: { windowMs: 1000, max: 1 }
    });
    gateway.start();

    await new Promise((resolve) => server.listen(0, resolve));
    port = server.address().port;
  });

  afterAll(async () => {
    await new Promise((resolve) => server.close(resolve));
    fs.rmSync(storeFile, { force: true });
  });

  it('acknowledges intent batches from leader connections and persists them', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/optimistic/ws?role=leader&site=test`);

    await new Promise((resolve, reject) => {
      ws.once('open', resolve);
      ws.once('error', reject);
    });

    const payload = {
      type: 'intent.batch',
      intents: [{ id: 'test-0001', intent: 'DEMO', payload: { foo: 'bar' }, meta: { clock: 1 } }]
    };

    const ack = await new Promise((resolve, reject) => {
      const handleMessage = (data) => {
        const parsed = JSON.parse(data.toString());
        if (parsed.type === 'hello' || parsed.type === 'replay') {
          return;
        }
        ws.off('message', handleMessage);
        resolve(parsed);
      };
      ws.on('message', handleMessage);
      ws.once('error', reject);
      ws.send(JSON.stringify(payload));
    });

    expect(ack.type).toBe('ack');
    expect(ack.intents).toHaveLength(1);
    expect(ack.intents[0].status).toBe('acked');
    expect(typeof ack.intents[0].logSeq).toBe('number');

    ws.close();

    const persisted = JSON.parse(fs.readFileSync(storeFile, 'utf8'));
    const entries = Array.isArray(persisted.entries) ? persisted.entries : [];
    expect(entries.length).toBeGreaterThan(0);
  });

  it('replays stored intents to reconnecting leaders', async () => {
    // Send another intent to ensure replay data exists
    const first = new WebSocket(`ws://127.0.0.1:${port}/optimistic/ws?role=leader&site=test`);
    await new Promise((resolve, reject) => {
      first.once('open', resolve);
      first.once('error', reject);
    });

    const payload = {
      type: 'intent.batch',
      intents: [{ id: 'test-0002', intent: 'DEMO', payload: { baz: 'qux' }, meta: { clock: 2 } }]
    };

    await new Promise((resolve, reject) => {
      const handleMessage = (data) => {
        const parsed = JSON.parse(data.toString());
        if (parsed.type === 'hello') return;
        if (parsed.type === 'ack') {
          first.off('message', handleMessage);
          resolve();
        }
      };
      first.on('message', handleMessage);
      first.once('error', reject);
      first.send(JSON.stringify(payload));
    });
    first.close();

    const second = new WebSocket(`ws://127.0.0.1:${port}/optimistic/ws?role=leader&site=test`);
    const replay = await new Promise((resolve, reject) => {
      const messages = [];
      const handleMessage = (data) => {
        const parsed = JSON.parse(data.toString());
        messages.push(parsed);
        if (parsed.type === 'replay') {
          second.off('message', handleMessage);
          resolve(parsed);
        }
      };
      second.on('message', handleMessage);
      second.once('error', reject);
      second.once('open', () => {
        // nothing, replay emitted automatically
      });
    });

    expect(replay.type).toBe('replay');
    expect(replay.intents.length).toBeGreaterThan(0);
    expect(replay.intents.some((intent) => intent.id === 'test-0002')).toBe(true);
    expect(typeof replay.cursor).toBe('number');
    second.close();
  });

  it('supports channel resync requests with cursors', async () => {
    store.append([{ id: 'resync-1', intent: 'DEMO', payload: {}, meta: { clock: 42 } }]);
    const eventHub = new OptimisticEventHub();
    const registry = new ChannelRegistry({ intentStore: store });
    registry.registerChannel('global', {
      load: ({ intentStore }) => intentStore.entries()
    });
    const gateway = new SyncGateway({ server, intentStore: store, eventHub, channelRegistry: registry });

    const connectionId = 'conn-resync';
    registry.attachConnection(connectionId, {
      send: () => {},
      context: { role: 'follower', site: 'test' }
    });
    await registry.subscribe(connectionId, { channel: 'global' });

    const sendSpy = vi.fn();
    await gateway.handleChannelResync({ send: sendSpy }, { connectionId, site: 'test' }, {
      channel: 'global',
      cursor: 0,
      limit: 5
    });
    expect(sendSpy).toHaveBeenCalled();
    const payload = JSON.parse(sendSpy.mock.calls[0][0]);
    expect(payload.type).toBe('channel.replay');
    expect(payload.status).toBe('ok');
    expect(Array.isArray(payload.intents)).toBe(true);
    expect(typeof payload.cursor).toBe('number');
  });

  it('handles channel command resend requests', async () => {
    store.append([{ id: 'cmd-1', intent: 'DEMO', payload: { value: 1 }, meta: { clock: 1 } }]);
    const eventHub = new OptimisticEventHub();
    const registry = new ChannelRegistry({ intentStore: store });
    registry.registerChannel('global', {
      load: ({ intentStore }) => intentStore.entries()
    });
    const gateway = new SyncGateway({ server, intentStore: store, eventHub, channelRegistry: registry });
    const replaySpy = vi.fn();
    registry.attachConnection('conn-cmd', {
      send: (payload) => replaySpy(payload),
      context: { role: 'follower', site: 'test' }
    });
    await registry.subscribe('conn-cmd', { channel: 'global', params: { scope: 'all' } });

    const sendSpy = vi.fn();
    await gateway.handleChannelCommand({ send: sendSpy }, { connectionId: 'conn-cmd', site: 'test' }, {
      channel: 'global',
      command: 'resend',
      params: { scope: 'all' },
      args: { reason: 'manual' }
    });

    const payloads = sendSpy.mock.calls.map(([raw]) => JSON.parse(raw));
    expect(payloads.some((msg) => msg.type === 'channel.command' && msg.status === 'ok')).toBe(true);
    const replayPayload = replaySpy.mock.calls.find(([payload]) => payload.type === 'channel.replay')?.[0];
    expect(replayPayload).toBeTruthy();
    expect(replayPayload.params).toEqual({ scope: 'all' });
  });

  it('broadcasts island invalidations to connected clients', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/optimistic/ws?role=follower&site=test`);
    await new Promise((resolve, reject) => {
      ws.once('open', resolve);
      ws.once('error', reject);
    });

    const messages = [];
    ws.on('message', (data) => {
      const parsed = JSON.parse(data.toString());
      if (parsed.type && parsed.type !== 'hello' && parsed.type !== 'replay') {
        messages.push(parsed);
      }
    });

    const payload = { islandId: 'product-inventory', timestamp: Date.now() };
    gateway.broadcast('island.invalidate', payload);

    await new Promise((resolve) => setTimeout(resolve, 50));
    ws.close();

    expect(messages.some((msg) => msg.type === 'island.invalidate' && msg.islandId === 'product-inventory')).toBe(true);
    const metrics = gateway.getMetrics();
    expect(metrics.broadcastsByType['island.invalidate']).toBeGreaterThan(0);
    expect(metrics.lastIslandInvalidateAt).toBeGreaterThan(0);
  });

  it('rate limits bursty channel subscriptions', async () => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/optimistic/ws?role=follower&site=test`);

    await new Promise((resolve, reject) => {
      ws.once('open', resolve);
      ws.once('error', reject);
    });

    const subscribe = () => new Promise((resolve, reject) => {
      const handler = (data) => {
        const parsed = JSON.parse(data.toString());
        if (parsed.type === 'hello' || parsed.type === 'replay') return;
        ws.off('message', handler);
        resolve(parsed);
      };
      ws.on('message', handler);
      ws.once('error', reject);
      ws.send(JSON.stringify({ type: 'channel.subscribe', channel: 'global', params: {} }));
    });

    const first = await subscribe();
    expect(first.status).toBe('ok');

    const limited = await subscribe();
    expect(limited.status).toBe('error');
    expect(limited.code).toBe('RATE_LIMITED');
    expect(typeof limited.retryAfterMs).toBe('number');
    ws.close();
  });

  it('rejects invalid channel payloads with INVALID_CONTRACT', async () => {
    const sendSpy = vi.fn();
    const fakeWs = { send: sendSpy };
    await gateway.handleMessage(fakeWs, { role: 'follower', connectionId: 'c-1', site: 'test' }, Buffer.from(JSON.stringify({
      type: 'channel.subscribe',
      channel: 42
    })));
    expect(sendSpy).toHaveBeenCalled();
    const frame = JSON.parse(sendSpy.mock.calls[0][0]);
    expect(frame.type).toBe('error');
    expect(frame.code).toBe('INVALID_CONTRACT');
  });

  it('rejects invalid ping payloads with INVALID_CONTRACT', async () => {
    const sendSpy = vi.fn();
    const fakeWs = { send: sendSpy };
    await gateway.handleMessage(fakeWs, { role: 'follower', connectionId: 'c-2', site: 'test' }, Buffer.from(JSON.stringify({
      type: 'ping',
      extra: true
    })));
    expect(sendSpy).toHaveBeenCalled();
    const frame = JSON.parse(sendSpy.mock.calls[0][0]);
    expect(frame.type).toBe('error');
    expect(frame.code).toBe('INVALID_CONTRACT');
  });

  it('rejects invalid channel.resync payloads with INVALID_CONTRACT', async () => {
    const sendSpy = vi.fn();
    const fakeWs = { send: sendSpy };
    await gateway.handleMessage(fakeWs, { role: 'follower', connectionId: 'c-3', site: 'test' }, Buffer.from(JSON.stringify({
      type: 'channel.resync',
      channel: 'global',
      cursor: -1
    })));
    expect(sendSpy).toHaveBeenCalled();
    const frame = JSON.parse(sendSpy.mock.calls[0][0]);
    expect(frame.type).toBe('error');
    expect(frame.code).toBe('INVALID_CONTRACT');
  });

  it('rejects invalid channel.command payloads with INVALID_CONTRACT', async () => {
    const sendSpy = vi.fn();
    const fakeWs = { send: sendSpy };
    await gateway.handleMessage(fakeWs, { role: 'follower', connectionId: 'c-4', site: 'test' }, Buffer.from(JSON.stringify({
      type: 'channel.command',
      channel: 'global',
      command: {}
    })));
    expect(sendSpy).toHaveBeenCalled();
    const frame = JSON.parse(sendSpy.mock.calls[0][0]);
    expect(frame.type).toBe('error');
    expect(frame.code).toBe('INVALID_CONTRACT');
  });

  it('falls back to intent-store snapshot when backend subscribe is unsupported', async () => {
    store.append([{ id: 'fallback-0001', intent: 'DEMO', payload: {}, meta: { clock: 7, channels: ['global'] } }], { site: 'test', connectionId: 'seed' });
    const eventHub = new OptimisticEventHub();
    const registry = new ChannelRegistry({ intentStore: store });
    registry.registerChannel('global', {
      load: ({ intentStore }) => intentStore.entries()
    });
    const gateway = new SyncGateway({
      server,
      intentStore: store,
      eventHub,
      channelRegistry: registry,
      backendClient: {
        supports: (capability) => capability === 'subscribe',
        subscribe: async () => ({ status: 'error', code: 'NOT_SUPPORTED' })
      }
    });
    const sent = [];
    const context = { connectionId: 'conn-fallback', site: 'test', role: 'follower', user: { id: 'u-f', role: 'user' } };
    registry.attachConnection(context.connectionId, {
      send(payload) {
        sent.push(payload);
      },
      context
    });
    await gateway.handleChannelSubscribe({ send: (raw) => sent.push(JSON.parse(raw)) }, context, {
      type: 'channel.subscribe',
      channel: 'global',
      params: {}
    });

    const snapshot = sent.find((entry) => entry.type === 'channel.snapshot');
    expect(snapshot).toBeTruthy();
    expect(Array.isArray(snapshot.intents)).toBe(true);
    expect(snapshot.intents.some((entry) => entry.id === 'fallback-0001')).toBe(true);
  });
});
