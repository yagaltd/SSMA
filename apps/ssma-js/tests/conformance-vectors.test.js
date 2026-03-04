import { afterEach, describe, expect, it } from 'vitest';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { URL, fileURLToPath } from 'node:url';
import { IntentStore } from '../src/services/optimistic/IntentStore.js';
import { OptimisticEventHub } from '../src/services/optimistic/OptimisticEventHub.js';
import { SyncGateway } from '../src/services/optimistic/SyncGateway.js';
import { ChannelRegistry } from '../src/services/optimistic/ChannelRegistry.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const vectorsDir = path.resolve(__dirname, '../../../packages/ssma-protocol/vectors');

function readVector(name) {
  return JSON.parse(fs.readFileSync(path.join(vectorsDir, `${name}.json`), 'utf8'));
}

function createMockWs() {
  const frames = [];
  return {
    frames,
    bufferedAmount: 0,
    send(payload) {
      const parsed = typeof payload === 'string' ? JSON.parse(payload) : payload;
      frames.push(parsed);
    },
    on() {},
    close() {}
  };
}

function frameTypes(frames) {
  return frames.map((frame) => frame.type);
}

describe('protocol conformance vectors', () => {
  const cleanup = [];

  afterEach(() => {
    while (cleanup.length) {
      fs.rmSync(cleanup.pop(), { force: true, recursive: true });
    }
  });

  function createGateway({ requireAuthForWrites = false, channelRateLimit } = {}) {
    const filePath = path.join(os.tmpdir(), `ssma-vector-${Date.now()}-${Math.random()}.json`);
    cleanup.push(filePath);
    const store = new IntentStore({ filePath, maxEntries: 100 });
    const hub = new OptimisticEventHub();
    const channels = new ChannelRegistry({ intentStore: store });
    channels.registerChannel('global', {
      load: ({ intentStore }) => intentStore.entriesAfter(0, { limit: 200, channels: ['global'] })
    });
    const gateway = new SyncGateway({
      intentStore: store,
      eventHub: hub,
      channelRegistry: channels,
      requireAuthForWrites,
      channelRateLimit
    });
    return { gateway, channels, store };
  }

  it('ws_handshake vector', async () => {
    const vector = readVector('ws_handshake');
    const { gateway } = createGateway();
    const ws = createMockWs();
    const connectPath = vector.client[0].path;
    await gateway.handleConnection(
      ws,
      new URL(connectPath, 'http://localhost'),
      { headers: { host: 'localhost' }, socket: { remoteAddress: '127.0.0.1' } }
    );

    expect(frameTypes(ws.frames).slice(0, 2)).toEqual(['hello', 'replay']);
    expect(ws.frames[0].subprotocol).toBe(vector.server[0].subprotocol);
  });

  it('intent_batch_ack vector', async () => {
    const vector = readVector('intent_batch_ack');
    const { gateway } = createGateway();
    const ws = createMockWs();
    await gateway.handleIntentBatch(ws, {
      role: 'leader',
      site: 'default',
      connectionId: 'conn-1',
      user: { id: 'u-1', role: 'user' },
      ip: '127.0.0.1',
      userAgent: 'vitest'
    }, vector.client[0]);

    const ack = ws.frames.find((frame) => frame.type === 'ack');
    expect(ack).toBeTruthy();
    expect(ack.intents[0].id).toBe(vector.server[0].intents[0].id);
    expect(ack.intents[0].status).toBe(vector.server[0].intents[0].status);
    expect(typeof ack.intents[0].logSeq).toBe('number');
  });

  it('channel_subscribe_snapshot vector', async () => {
    const vector = readVector('channel_subscribe_snapshot');
    const { gateway, channels } = createGateway();
    const ws = createMockWs();
    const context = { connectionId: 'conn-2', site: 'default', role: 'follower', user: { id: 'u-2', role: 'user' } };
    channels.attachConnection(context.connectionId, {
      send(payload) {
        ws.send(JSON.stringify(payload));
      },
      context
    });

    await gateway.handleChannelSubscribe(ws, context, vector.client[0]);
    const types = frameTypes(ws.frames);
    expect(types).toContain('channel.ack');
    expect(types).toContain('channel.snapshot');
  });

  it('rate_limit_channel_subscribe vector', async () => {
    const vector = readVector('rate_limit_channel_subscribe');
    const { gateway, channels } = createGateway({ channelRateLimit: { windowMs: 10_000, max: 1 } });
    const ws = createMockWs();
    const context = { connectionId: 'conn-3', site: 'default', role: 'follower', user: { id: 'u-3', role: 'user' }, ip: '127.0.0.1' };
    channels.attachConnection(context.connectionId, {
      send(payload) {
        ws.send(JSON.stringify(payload));
      },
      context
    });

    await gateway.handleChannelSubscribe(ws, context, vector.client[0]);
    await gateway.handleChannelSubscribe(ws, context, vector.client[1]);

    const acks = ws.frames.filter((frame) => frame.type === 'channel.ack');
    expect(acks[0].status).toBe(vector.server[0].status);
    expect(acks[1].status).toBe(vector.server[1].status);
    expect(acks[1].code).toBe(vector.server[1].code);
  });

  it('unauthorized_ws_reject vector', async () => {
    const vector = readVector('unauthorized_ws_reject');
    const { gateway } = createGateway({ requireAuthForWrites: true });
    const ws = createMockWs();
    await gateway.handleIntentBatch(ws, {
      role: 'leader',
      site: 'default',
      connectionId: 'conn-4',
      user: null,
      ip: '127.0.0.1',
      userAgent: 'vitest'
    }, vector.client[0]);

    expect(ws.frames).toHaveLength(1);
    expect(ws.frames[0].type).toBe('error');
    expect(ws.frames[0].code).toBe(vector.server[0].code);
  });

  it('replay_window vector', async () => {
    const vector = readVector('replay_window');
    const { gateway, store } = createGateway();
    store.append([
      {
        id: 'i-replay-0002',
        intent: 'TODO_CREATE',
        payload: { id: 'todo-r', title: 'seed' },
        meta: { clock: Date.now(), channels: ['global'] }
      }
    ], { site: 'default', connectionId: 'conn-seed' });
    const ws = createMockWs();

    await gateway.handleConnection(
      ws,
      new URL(vector.client[0].path, 'http://localhost'),
      { headers: { host: 'localhost' }, socket: { remoteAddress: '127.0.0.1' } }
    );

    const replay = ws.frames.find((frame) => frame.type === 'replay');
    expect(replay).toBeTruthy();
    expect(Array.isArray(replay.intents)).toBe(true);
    expect(replay.cursor).toBeGreaterThanOrEqual(vector.server[0].cursor);
  });
});
