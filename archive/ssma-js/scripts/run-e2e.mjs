#!/usr/bin/env node
import { createServer } from '../src/app.js';
import { loadConfig } from '../src/config/env.js';

async function main() {
  const config = loadConfig();
  const server = createServer(config);

  await new Promise((resolve) => server.listen(0, resolve));
  const { port } = server.address();
  const baseUrl = `http://127.0.0.1:${port}`;

  try {
    await healthCheck(baseUrl);
    const login = await post(`${baseUrl}/auth/login`, { email: 'demo@ssma.local', password: 'demo-password' });
    if (!login.accessToken || !login.refreshToken) throw new Error('login missing tokens');

    const refresh = await post(`${baseUrl}/auth/refresh`, { refreshToken: login.refreshToken });
    if (!refresh.accessToken) throw new Error('refresh missing access token');

    const issued = await post(`${baseUrl}/auth/api-key/issue`, { email: 'demo@ssma.local' });
    const apiKeyLogin = await post(`${baseUrl}/auth/api-key/login`, { apiKey: issued.apiKey });
    if (!apiKeyLogin.accessToken) throw new Error('api key login missing token');

    const nonce = await post(`${baseUrl}/auth/hmac/nonce`, { intent: 'PUBLIC_FORM_SUBMIT' });
    if (!nonce.nonce) throw new Error('nonce missing');

    const batchId = `batch_${Date.now()}`;
    const logAck = await post(`${baseUrl}/logs/batch`, {
      batchId,
      sessionId: 'session-demo',
      userId: 'demo-user',
      source: 'csma-kit',
      meta: { platform: 'web', clientTime: Date.now() },
      entries: [
        {
          event: 'E2E_CLIENT_EVENT',
          level: 'info',
          message: 'E2E log entry',
          timestamp: Date.now()
        }
      ]
    });
    if (logAck.batchId !== batchId) throw new Error('log ack mismatch');

    const logHealth = await getJson(`${baseUrl}/logs/health`);
    if (!logHealth || !logHealth.store) throw new Error('missing log health');

    console.log('[run-e2e] all checks passed');
  } finally {
    server.close();
  }
}

async function healthCheck(baseUrl) {
  const res = await fetch(`${baseUrl}/health`);
  if (!res.ok) throw new Error('health check failed');
}

async function post(url, body) {
  const res = await fetch(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body)
  });
  const json = await res.json();
  if (!res.ok) {
    throw new Error(`Request failed ${res.status}: ${JSON.stringify(json)}`);
  }
  return json;
}

async function getJson(url) {
  const res = await fetch(url);
  const json = await res.json();
  if (!res.ok) {
    throw new Error(`Request failed ${res.status}: ${JSON.stringify(json)}`);
  }
  return json;
}

main().catch((error) => {
  console.error('[run-e2e] failed', error);
  process.exit(1);
});
