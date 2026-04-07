import { describe, it, expect, vi } from 'vitest';
import { EventEmitter } from 'node:events';
import { OptimisticEventHub } from '../src/services/optimistic/OptimisticEventHub.js';
import { SyncGateway } from '../src/services/optimistic/SyncGateway.js';

class MockSseResponse extends EventEmitter {
  constructor({ block = false } = {}) {
    super();
    this.block = block;
    this.chunks = [];
    this.statusCode = 200;
  }

  setHeader() {}
  flushHeaders() {}

  write(chunk) {
    const value = chunk.toString();
    this.chunks.push(value);
    return !this.block;
  }

  end() {
    this.emit('close');
  }
}

function createSseContext({ role = 'guest', islands, block = false } = {}) {
  return {
    res: new MockSseResponse({ block }),
    headers: { origin: 'http://localhost', 'user-agent': 'vitest' },
    state: { user: { role, id: `${role}-1` } },
    ip: '127.0.0.1',
    query: islands ? { islands: islands.join(',') } : {}
  };
}

function createWsStub({ bufferedAfterSend = 0 } = {}) {
  const ws = {
    sent: [],
    bufferedAmount: 0,
    readyState: 1,
    CLOSING: 2,
    CLOSED: 3,
    send(payload) {
      this.sent.push(payload);
      this.bufferedAmount = bufferedAfterSend;
    },
    close: vi.fn(function () {
      this.readyState = this.CLOSING;
    })
  };
  return ws;
}

describe('Hybrid transport metrics', () => {
  it('tracks SSE forwarding stats', () => {
    const hub = new OptimisticEventHub();
    hub.publish('noop', { ok: true });
    hub.publish('island.invalidate', { islandId: 'demo' });

    const metrics = hub.getMetrics();
    expect(metrics.eventsTotal).toBe(2);
    expect(metrics.eventsByType['island.invalidate']).toBe(1);
    expect(metrics.lastIslandInvalidateAt).toBeGreaterThan(0);
    expect(metrics.eventsByType.noop).toBe(1);
  });

  it('tracks WebSocket broadcast stats', () => {
    const gateway = new SyncGateway({ server: { on: () => {} }, intentStore: null, eventHub: null });
    gateway.broadcast('island.invalidate', { islandId: 'demo' });
    gateway.broadcast('island.invalidate', { islandId: 'demo-2' });

    const metrics = gateway.getMetrics();
    expect(metrics.broadcastsTotal).toBe(2);
    expect(metrics.broadcastsByType['island.invalidate']).toBe(2);
    expect(metrics.lastIslandInvalidateAt).toBeGreaterThan(0);
  });

  it('enforces SSE RBAC per island', () => {
    const hub = new OptimisticEventHub({
      transport: { islandAccess: { 'ops.dashboard': 'staff' } }
    });
    const guestCtx = createSseContext({ role: 'guest' });
    const staffCtx = createSseContext({ role: 'staff' });
    hub.addClient(guestCtx);
    hub.addClient(staffCtx);

    hub.publish('island.invalidate', { islandId: 'ops.dashboard', payload: {} });

    const guestEvents = guestCtx.res.chunks.join('');
    const staffEvents = staffCtx.res.chunks.join('');
    expect(guestEvents.includes('ops.dashboard')).toBe(false);
    expect(staffEvents.includes('ops.dashboard')).toBe(true);
  });

  it('drops slow SSE clients when queue overflows', () => {
    const hub = new OptimisticEventHub({
      transport: {
        sse: { maxQueueBytes: 32, drainTimeoutMs: 5, retryMs: 5 },
        islandAccess: { 'product-inventory': 'guest' }
      }
    });
    const slowCtx = createSseContext({ role: 'guest', block: true });
    hub.addClient(slowCtx);
    hub.publish('island.invalidate', { islandId: 'product-inventory', value: 1 });
    hub.publish('island.invalidate', { islandId: 'product-inventory', value: 2 });
    const metrics = hub.getMetrics();
    expect(metrics.slowClientDrops).toBeGreaterThanOrEqual(1);
  });

  it('filters WebSocket broadcasts by island role', () => {
    const gateway = new SyncGateway({ transport: { islandAccess: { 'ops.dashboard': 'staff' } } });
    const guestWs = createWsStub();
    const staffWs = createWsStub();
    gateway.connections.set('guest', { ws: guestWs, context: { role: 'guest', user: { role: 'guest' } } });
    gateway.connections.set('staff', { ws: staffWs, context: { role: 'staff', user: { role: 'staff' } } });

    gateway.broadcast('island.invalidate', { islandId: 'ops.dashboard' });

    expect(guestWs.sent.length).toBe(0);
    expect(staffWs.sent.length).toBe(1);
  });

  it('drops slow WebSocket consumers when buffered amount grows', () => {
    const gateway = new SyncGateway({ transport: { ws: { maxBufferedBytes: 10 } } });
    const ws = createWsStub({ bufferedAfterSend: 50 });
    gateway.connections.set('conn', { ws, context: { connectionId: 'conn', role: 'follower', user: { role: 'guest' } } });

    gateway.broadcast('island.invalidate', { islandId: 'product-inventory' });

    expect(ws.close).toHaveBeenCalled();
    const metrics = gateway.getMetrics();
    expect(metrics.backpressureEvents).toBeGreaterThanOrEqual(1);
    expect(metrics.slowConsumerDrops).toBeGreaterThanOrEqual(1);
  });
});
