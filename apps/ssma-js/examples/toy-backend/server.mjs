import http from 'node:http';
import { randomUUID } from 'node:crypto';

function json(res, status, payload) {
  res.statusCode = status;
  res.setHeader('content-type', 'application/json');
  res.end(JSON.stringify(payload));
}

async function readBody(req) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  const raw = Buffer.concat(chunks).toString('utf8');
  if (!raw) return {};
  try {
    return JSON.parse(raw);
  } catch {
    return {};
  }
}

export function createToyBackendServer({ ssmaEventsUrl = '', ssmaToken = '' } = {}) {
  const todos = new Map();
  const applyCountByIntent = new Map();
  const seenIntentIds = new Set();

  const server = http.createServer(async (req, res) => {
    const url = new URL(req.url || '/', `http://${req.headers.host || 'localhost'}`);

    if ((req.method === 'GET' || req.method === 'POST') && url.pathname === '/health') {
      return json(res, 200, { status: 'ok', service: 'toy-backend', todos: todos.size });
    }

    if (req.method === 'GET' && url.pathname === '/metrics') {
      const applies = Array.from(applyCountByIntent.entries()).map(([id, count]) => ({ id, count }));
      return json(res, 200, { status: 'ok', applyCountByIntent: applies });
    }

    if (req.method === 'POST' && url.pathname === '/apply-intents') {
      const body = await readBody(req);
      const intents = Array.isArray(body?.intents) ? body.intents : [];
      const context = body?.context || {};
      const results = [];
      const events = [];

      for (const intentEntry of intents) {
        const intentId = intentEntry?.id || randomUUID();
        const seen = seenIntentIds.has(intentId);
        applyCountByIntent.set(intentId, (applyCountByIntent.get(intentId) || 0) + 1);

        if (seen) {
          results.push({ id: intentId, status: 'acked', code: 'IDEMPOTENT_REPLAY' });
          continue;
        }

        seenIntentIds.add(intentId);
        const payload = intentEntry?.payload || {};
        const now = Date.now();
        let status = 'acked';

        switch (intentEntry?.intent) {
          case 'TODO_CREATE': {
            const id = payload.id || `todo-${randomUUID().slice(0, 8)}`;
            todos.set(id, {
              id,
              title: payload.title || 'Untitled',
              completed: false,
              updatedAt: now
            });
            break;
          }
          case 'TODO_UPDATE': {
            const existing = todos.get(payload.id);
            if (!existing) {
              status = 'conflict';
              break;
            }
            todos.set(payload.id, {
              ...existing,
              title: typeof payload.title === 'string' ? payload.title : existing.title,
              updatedAt: now
            });
            break;
          }
          case 'TODO_DELETE': {
            if (!todos.has(payload.id)) {
              status = 'conflict';
              break;
            }
            todos.delete(payload.id);
            break;
          }
          case 'TODO_TOGGLE': {
            const existing = todos.get(payload.id);
            if (!existing) {
              status = 'conflict';
              break;
            }
            todos.set(payload.id, {
              ...existing,
              completed: !existing.completed,
              updatedAt: now
            });
            break;
          }
          default:
            status = 'rejected';
        }

        const event = {
          eventId: `evt-${intentId}`,
          type: 'todo.updated',
          reason: 'backend-apply',
          site: context.site || 'default',
          timestamp: now,
          islandId: 'product-inventory',
          intents: [
            {
              id: intentId,
              intent: intentEntry.intent,
              payload: intentEntry.payload || {},
              meta: intentEntry.meta || {},
              insertedAt: now,
              logSeq: intentEntry.logSeq || now
            }
          ]
        };

        if (status === 'acked') {
          events.push(event);
        }

        results.push({
          id: intentId,
          status,
          events: status === 'acked' ? [event] : []
        });
      }

      if (ssmaEventsUrl && events.length) {
        try {
          await fetch(ssmaEventsUrl, {
            method: 'POST',
            headers: {
              'content-type': 'application/json',
              ...(ssmaToken ? { 'x-ssma-backend-token': ssmaToken } : {})
            },
            body: JSON.stringify({ events })
          });
        } catch {
          // Non-fatal: the response still carries events for direct gateway ingest.
        }
      }

      return json(res, 200, { results, events });
    }

    if ((req.method === 'POST' || req.method === 'GET') && url.pathname.startsWith('/query/')) {
      const name = decodeURIComponent(url.pathname.slice('/query/'.length));
      if (name === 'todos') {
        const rows = Array.from(todos.values()).sort((a, b) => a.id.localeCompare(b.id));
        return json(res, 200, { status: 'ok', data: { todos: rows } });
      }
      return json(res, 404, { error: 'UNKNOWN_QUERY' });
    }

    if (req.method === 'POST' && url.pathname === '/subscribe') {
      const body = await readBody(req);
      const channel = String(body?.channel || '');
      if (channel !== 'global') {
        return json(res, 200, {
          status: 'error',
          code: 'NOT_SUPPORTED',
          snapshot: [],
          cursor: 0
        });
      }
      const snapshot = Array.from(todos.values())
        .sort((a, b) => a.id.localeCompare(b.id))
        .map((todo) => ({
          id: `snapshot-${todo.id}`,
          intent: 'TODO_SNAPSHOT',
          payload: todo,
          meta: { channels: ['global'], source: 'backend' },
          insertedAt: todo.updatedAt,
          logSeq: todo.updatedAt
        }));
      const cursor = snapshot.length ? snapshot[snapshot.length - 1].logSeq : 0;
      return json(res, 200, { status: 'ok', snapshot, cursor });
    }

    json(res, 404, { error: 'NOT_FOUND' });
  });

  return server;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const port = Number(process.env.TOY_BACKEND_PORT || 6060);
  const ssmaBase = process.env.SSMA_BASE_URL || 'http://127.0.0.1:5050';
  const ssmaToken = process.env.SSMA_BACKEND_INTERNAL_TOKEN || '';
  const server = createToyBackendServer({
    ssmaEventsUrl: `${ssmaBase.replace(/\/$/, '')}/internal/backend/events`,
    ssmaToken
  });
  server.listen(port, () => {
    console.log(`[toy-backend] listening on ${port}`);
  });
}
