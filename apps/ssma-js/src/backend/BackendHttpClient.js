export class BackendHttpClient {
  constructor({ baseUrl, timeoutMs = 5000 } = {}) {
    this.baseUrl = String(baseUrl || '').replace(/\/$/, '');
    this.timeoutMs = timeoutMs;
    this.capabilities = Object.freeze({
      applyIntents: true,
      query: true,
      subscribe: true,
      health: true
    });
  }

  isConfigured() {
    return Boolean(this.baseUrl);
  }

  getCapabilities() {
    return { ...this.capabilities };
  }

  supports(capability) {
    return Boolean(this.capabilities?.[capability]);
  }

  async applyIntents(intents, ctx = {}) {
    if (!this.isConfigured()) {
      return { results: intents.map((intent) => ({ id: intent.id, status: 'acked' })), events: [] };
    }

    return this.#request('/apply-intents', {
      method: 'POST',
      body: {
        intents,
        context: this.#sanitizeContext(ctx)
      }
    });
  }

  async query(name, payload = {}, ctx = {}) {
    if (!this.isConfigured()) {
      return { status: 'ok', data: null };
    }

    const encoded = encodeURIComponent(String(name || 'default'));
    return this.#request(`/query/${encoded}`, {
      method: 'POST',
      body: {
        payload,
        context: this.#sanitizeContext(ctx)
      }
    });
  }

  async subscribe(channel, params = {}, ctx = {}) {
    if (!this.isConfigured()) {
      return { status: 'ok', snapshot: [], cursor: 0 };
    }

    try {
      return await this.#request('/subscribe', {
        method: 'POST',
        body: {
          channel,
          params,
          context: this.#sanitizeContext(ctx)
        }
      });
    } catch (error) {
      if (error.status === 404 || error.status === 501) {
        return {
          status: 'error',
          code: 'NOT_SUPPORTED',
          snapshot: [],
          cursor: 0
        };
      }
      throw error;
    }
  }

  async health(ctx = {}) {
    if (!this.isConfigured()) {
      return { status: 'ok', backend: 'unconfigured' };
    }
    try {
      return await this.#request('/health', {
        method: 'POST',
        body: {
          context: this.#sanitizeContext(ctx)
        }
      });
    } catch (error) {
      if (error.status !== 404 && error.status !== 405) {
        throw error;
      }
    }
    return this.#request('/health', { method: 'GET' });
  }

  async #request(path, { method = 'GET', body } = {}) {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);

    try {
      const response = await fetch(`${this.baseUrl}${path}`, {
        method,
        headers: {
          'content-type': 'application/json'
        },
        body: body ? JSON.stringify(body) : undefined,
        signal: controller.signal
      });

      const json = await response.json().catch(() => ({}));
      if (!response.ok) {
        const error = new Error(json?.error || `Backend request failed with ${response.status}`);
        error.status = response.status;
        error.details = json;
        throw error;
      }
      return json;
    } finally {
      clearTimeout(timer);
    }
  }

  #sanitizeContext(ctx = {}) {
    return {
      site: ctx.site || 'default',
      connectionId: ctx.connectionId || null,
      ip: ctx.ip || null,
      userAgent: ctx.userAgent || null,
      user: ctx.user
        ? {
            id: ctx.user.id || null,
            role: ctx.user.role || 'user'
          }
        : null
    };
  }
}
