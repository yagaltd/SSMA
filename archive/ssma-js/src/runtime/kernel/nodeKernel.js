import http from 'node:http';
import { URL } from 'node:url';

const METHODS_WITH_BODY = new Set(['POST', 'PUT', 'PATCH', 'DELETE']);
const JSON_TYPES = ['application/json', 'text/json'];

export function createNodeKernel(config = {}) {
  const middlewares = [];
  const routes = new Map();

  const server = http.createServer(async (req, res) => {
    const ctx = createContext(req, res, config);

    try {
      await parseBody(ctx);
      await runPipeline(ctx);
    } catch (error) {
      handleError(ctx, error);
    }
  });

  async function runPipeline(ctx) {
    const pipeline = [...middlewares, dispatchRoute];
    let index = -1;

    const dispatch = async (i) => {
      if (ctx.responded) return;
      if (i <= index) {
        throw new Error('next() called multiple times');
      }
      index = i;
      const fn = pipeline[i];
      if (!fn) return;
      await fn(ctx, () => dispatch(i + 1));
    };

    await dispatch(0);
  }

  async function dispatchRoute(ctx) {
    const table = routes.get(ctx.method);
    if (!table || table.length === 0) {
      ctx.json(404, { error: 'Not Found' });
      return;
    }
    for (const route of table) {
      const params = route.match(ctx.path);
      if (params) {
        ctx.params = params;
        await route.handler(ctx);
        return;
      }
    }
    ctx.json(404, { error: 'Not Found' });
  }

  function use(fn) {
    middlewares.push(fn);
  }

  function route(method, path, handler) {
    const normalizedMethod = method.toUpperCase();
    const matcher = createMatcher(path);
    const bucket = routes.get(normalizedMethod) || [];
    bucket.push({ handler, match: matcher });
    routes.set(normalizedMethod, bucket);
  }

  return { server, use, route };
}

function createContext(req, res, config) {
  const url = new URL(req.url || '/', `http://${req.headers.host || 'localhost'}`);
  return {
    req,
    res,
    config,
    method: (req.method || 'GET').toUpperCase(),
    url,
    path: url.pathname,
    query: Object.fromEntries(url.searchParams.entries()),
    headers: req.headers,
    ip: (req.headers['x-forwarded-for']?.split(',')[0] || req.socket.remoteAddress || '').trim(),
    state: Object.create(null),
    params: Object.create(null),
    responded: false,
    rawBody: '',
    body: undefined,
    json(status, payload) {
      if (this.responded) return;
      this.res.statusCode = status;
      this.res.setHeader('Content-Type', 'application/json');
      this.res.end(JSON.stringify(payload));
      this.responded = true;
    },
    text(status, message) {
      if (this.responded) return;
      this.res.statusCode = status;
      this.res.setHeader('Content-Type', 'text/plain');
      this.res.end(message);
      this.responded = true;
    }
  };
}

function createMatcher(path) {
  const normalized = normalizePath(path);
  if (!normalized.includes(':')) {
    return (incomingPath) => (normalizePath(incomingPath) === normalized ? Object.create(null) : null);
  }

  const segments = normalized.split('/').filter(Boolean);
  return (incomingPath) => {
    const incoming = normalizePath(incomingPath).split('/').filter(Boolean);
    if (segments.length !== incoming.length) {
      return null;
    }
    const params = Object.create(null);
    for (let i = 0; i < segments.length; i += 1) {
      const segment = segments[i];
      const value = incoming[i];
      if (segment.startsWith(':')) {
        params[segment.slice(1)] = decodeURIComponent(value);
        continue;
      }
      if (segment !== value) {
        return null;
      }
    }
    return params;
  };
}

function normalizePath(path) {
  if (!path) return '/';
  const normalized = path.startsWith('/') ? path : `/${path}`;
  return normalized.replace(/\/+$/, '') || '/';
}

async function parseBody(ctx) {
  if (!METHODS_WITH_BODY.has(ctx.method)) {
    return;
  }

  const chunks = [];
  let totalLength = 0;
  const limit = 1 * 1024 * 1024; // 1MB

  for await (const chunk of ctx.req) {
    totalLength += chunk.length;
    if (totalLength > limit) {
      const error = new Error('Payload too large');
      error.status = 413;
      throw error;
    }
    chunks.push(chunk);
  }

  const raw = Buffer.concat(chunks).toString('utf-8');
  ctx.rawBody = raw;
  const contentType = ctx.headers['content-type'] || '';

  if (!raw) {
    ctx.body = {};
    return;
  }

  if (JSON_TYPES.some((type) => contentType.includes(type)) || contentType === '') {
    try {
      ctx.body = JSON.parse(raw);
    } catch (error) {
      const parseError = new Error('Invalid JSON payload');
      parseError.status = 400;
      throw parseError;
    }
  } else {
    ctx.body = raw;
  }
}

function handleError(ctx, error) {
  if (ctx.responded) {
    console.error('[kernel] error after response', error);
    return;
  }
  const status = typeof error.status === 'number' ? error.status : 500;
  const payload = {
    error: error.message || 'Internal server error',
    code: error.code || 'INTERNAL_ERROR'
  };
  if (error.details) {
    payload.details = error.details;
  }
  ctx.json(status, payload);
}
