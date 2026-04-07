import { describe, it, expect, vi, afterEach } from 'vitest';
import { BackendHttpClient } from '../src/backend/BackendHttpClient.js';

describe('BackendHttpClient', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('exposes declared capabilities', () => {
    const client = new BackendHttpClient({ baseUrl: 'http://backend.local' });
    expect(client.supports('applyIntents')).toBe(true);
    expect(client.supports('query')).toBe(true);
    expect(client.supports('subscribe')).toBe(true);
    expect(client.supports('health')).toBe(true);
  });

  it('queries backend using POST /query/:name', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ status: 'ok', data: { rows: [] } })
    });
    const client = new BackendHttpClient({ baseUrl: 'http://backend.local' });

    const result = await client.query('todos', { limit: 10 }, { site: 's1' });
    expect(result.status).toBe('ok');
    const [url, options] = fetchSpy.mock.calls[0];
    expect(url).toBe('http://backend.local/query/todos');
    expect(options.method).toBe('POST');
    expect(JSON.parse(options.body).payload).toEqual({ limit: 10 });
  });

  it('sends canonical backend context shape', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ results: [] })
    });
    const client = new BackendHttpClient({ baseUrl: 'http://backend.local' });

    await client.applyIntents(
      [{ id: 'i-1', intent: 'TODO_CREATE', payload: { id: 'todo-1' }, meta: {} }],
      {
        site: 'default',
        connectionId: 'conn-1',
        ip: '127.0.0.1',
        userAgent: 'vitest',
        user: { id: 'user-1', role: 'staff' }
      }
    );

    const [, options] = fetchSpy.mock.calls[0];
    expect(JSON.parse(options.body).context).toEqual({
      site: 'default',
      connectionId: 'conn-1',
      ip: '127.0.0.1',
      userAgent: 'vitest',
      user: { id: 'user-1', role: 'staff' }
    });
  });

  it('falls back to GET /health when POST health is unsupported', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch')
      .mockResolvedValueOnce({
        ok: false,
        status: 405,
        json: async () => ({ error: 'METHOD_NOT_ALLOWED' })
      })
      .mockResolvedValueOnce({
        ok: true,
        status: 200,
        json: async () => ({ status: 'ok' })
      });

    const client = new BackendHttpClient({ baseUrl: 'http://backend.local' });
    const health = await client.health();
    expect(health.status).toBe('ok');
    expect(fetchSpy).toHaveBeenCalledTimes(2);
    expect(fetchSpy.mock.calls[0][0]).toBe('http://backend.local/health');
    expect(fetchSpy.mock.calls[0][1].method).toBe('POST');
    expect(fetchSpy.mock.calls[1][0]).toBe('http://backend.local/health');
    expect(fetchSpy.mock.calls[1][1].method).toBe('GET');
  });

  it('returns NOT_SUPPORTED for subscribe endpoint 404/501', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: false,
      status: 404,
      json: async () => ({ error: 'NOT_FOUND' })
    });
    const client = new BackendHttpClient({ baseUrl: 'http://backend.local' });

    const result = await client.subscribe('global', {}, { site: 'default' });
    expect(result.status).toBe('error');
    expect(result.code).toBe('NOT_SUPPORTED');
  });
});
