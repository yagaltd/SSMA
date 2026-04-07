import { describe, it, expect, beforeEach } from 'vitest';
import { ChannelRegistry } from '../src/services/optimistic/ChannelRegistry.js';

describe('ChannelRegistry', () => {
  let registry;
  let sent;

  beforeEach(() => {
    sent = [];
    const entries = [
      { id: 'one', logSeq: 1, meta: { channels: ['global'], reasons: ['replay', 'channel:global'] } }
    ];
    const intentStore = {
      entriesSince: () => entries,
      entriesAfter: () => entries,
      latestCursor: () => entries[entries.length - 1].logSeq
    };
    registry = new ChannelRegistry({ intentStore, replayWindowMs: 10 });
    registry.registerChannel('global', {});
    registry.attachConnection('conn-1', {
      send: (payload) => sent.push(payload),
      context: { role: 'follower' }
    });
  });

  it('subscribes and emits param-scoped invalidations', async () => {
    const response = await registry.subscribe('conn-1', { channel: 'global', params: { region: 'us' } });
    expect(response.status).toBe('ok');
    registry.broadcast([
      { id: 'broadcast', meta: { channels: ['global'], reasons: ['channel:global'] } }
    ]);
    const invalidate = sent.find((payload) => payload.type === 'channel.invalidate');
    expect(invalidate).toBeTruthy();
    expect(invalidate.params).toEqual({ region: 'us' });
  });

  it('supports resend command and typed close reasons', async () => {
    await registry.subscribe('conn-1', { channel: 'global', params: { view: 'all' } });
    await registry.command('conn-1', { channel: 'global', params: { view: 'all' }, command: 'resend', args: { reason: 'manual' } });
    const replay = sent.find((payload) => payload.type === 'channel.replay');
    expect(replay).toBeTruthy();
    expect(replay.params).toEqual({ view: 'all' });

    registry.unsubscribe('conn-1', { channel: 'global', params: { view: 'all' } });
    const close = sent.find((payload) => payload.type === 'channel.close');
    expect(close).toBeTruthy();
    expect(close.code).toBe('CLIENT_UNSUBSCRIBED');
  });

  it('returns close metadata when access is denied', async () => {
    registry.registerChannel('secure', { access: () => false });
    const response = await registry.subscribe('conn-1', { channel: 'secure' });
    expect(response.status).toBe('error');
    expect(response.close).toEqual({ code: 'ACCESS_DENIED', reason: 'Access denied' });
  });
});
