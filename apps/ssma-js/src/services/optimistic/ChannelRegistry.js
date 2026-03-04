export class ChannelRegistry {
  constructor({ intentStore, replayWindowMs = 5 * 60 * 1000 } = {}) {
    this.intentStore = intentStore;
    this.replayWindowMs = replayWindowMs;
    this.channels = new Map();
    this.connections = new Map();
    this.channelSubscriptions = new Map();
  }

  registerChannel(id, config = {}) {
    if (!id) {
      throw new Error('[ChannelRegistry] Missing channel id');
    }
    this.channels.set(id, {
      access: config.access,
      load: config.load,
      unload: config.unload,
      filter: config.filter,
      resend: config.resend,
      commands: config.commands || {},
      metadata: config.metadata || {}
    });
  }

  attachConnection(connectionId, transport) {
    if (!connectionId || !transport?.send) return;
    this.connections.set(connectionId, {
      send: transport.send,
      context: transport.context || {},
      subscriptions: new Map()
    });
  }

  detachConnection(connectionId) {
    const connection = this.connections.get(connectionId);
    if (!connection) return;
    for (const [subscriptionKey, details] of connection.subscriptions.entries()) {
      this._removeSubscription(connectionId, details.channel, subscriptionKey, false, {
        code: 'CONNECTION_CLOSED',
        reason: 'Connection closed'
      });
    }
    this.connections.delete(connectionId);
  }

  async subscribe(connectionId, { channel, params = {}, filter = null } = {}) {
    const connection = this.connections.get(connectionId);
    if (!connection) {
      return { status: 'error', code: 'UNKNOWN_CONNECTION', close: { code: 'UNKNOWN_CONNECTION', reason: 'Connection not found' } };
    }
    const spec = this.channels.get(channel);
    if (!spec) {
      return { status: 'error', code: 'UNKNOWN_CHANNEL', close: { code: 'UNKNOWN_CHANNEL', reason: 'Channel not registered' } };
    }
    const context = { params, connection: connection.context };
    if (spec.access && (await spec.access(params, context)) === false) {
      return { status: 'error', code: 'ACCESS_DENIED', close: { code: 'ACCESS_DENIED', reason: 'Access denied' } };
    }
    const subscriptionKey = this._subscriptionKey(channel, params);
    if (connection.subscriptions.has(subscriptionKey)) {
      return { status: 'ok', code: 'ALREADY_SUBSCRIBED', channel, params };
    }
    connection.subscriptions.set(subscriptionKey, {
      channel,
      params,
      subscribedAt: Date.now(),
      filter: filter || null
    });

    if (!this.channelSubscriptions.has(channel)) {
      this.channelSubscriptions.set(channel, new Map());
    }
    const channelMap = this.channelSubscriptions.get(channel);
    if (!channelMap.has(connectionId)) {
      channelMap.set(connectionId, new Set());
    }
    channelMap.get(connectionId).add(subscriptionKey);

    const snapshot = await this._loadSnapshot(spec, { channel, params, connection: connection.context, filter });
    return {
      status: 'ok',
      channel,
      intents: snapshot.entries,
      cursor: snapshot.cursor,
      params,
      metadata: spec.metadata
    };
  }

  unsubscribe(connectionId, { channel, params = {} } = {}) {
    const subscriptionKey = this._subscriptionKey(channel, params);
    return this._removeSubscription(connectionId, channel, subscriptionKey, true, {
      code: 'CLIENT_UNSUBSCRIBED',
      reason: 'Client requested unsubscribe'
    });
  }

  async resync(connectionId, { channel, cursor = 0, limit = 200, params = {} } = {}) {
    const connection = this.connections.get(connectionId);
    if (!connection) {
      return { status: 'error', code: 'UNKNOWN_CONNECTION' };
    }
    const spec = this.channels.get(channel);
    if (!spec) {
      return { status: 'error', code: 'UNKNOWN_CHANNEL' };
    }
    const subscription = this._getSubscription(connectionId, channel, params);
    const snapshot = await this._loadSnapshot(spec, {
      channel,
      params,
      cursor,
      limit,
      connection: connection.context,
      filter: subscription?.filter || null
    });
    return {
      status: 'ok',
      channel,
      cursor: snapshot.cursor,
      intents: snapshot.entries,
      params
    };
  }

  async command(connectionId, { channel, params = {}, command, args = {} } = {}) {
    const connection = this.connections.get(connectionId);
    if (!connection) {
      return { status: 'error', code: 'UNKNOWN_CONNECTION' };
    }
    const spec = this.channels.get(channel);
    if (!spec) {
      return { status: 'error', code: 'UNKNOWN_CHANNEL' };
    }
    const subscription = this._getSubscription(connectionId, channel, params);
    if (!subscription) {
      return { status: 'error', code: 'NOT_SUBSCRIBED' };
    }
    switch (command) {
      case 'filter':
        subscription.filter = args?.filter || null;
        return { status: 'ok', channel, command: 'filter', params, filter: subscription.filter };
      case 'resend': {
        const snapshot = await this._resendSubscription(connectionId, subscription, spec, args.reason);
        if (!snapshot) {
          return { status: 'error', code: 'RESEND_FAILED' };
        }
        return { status: 'ok', channel, command: 'resend', params };
      }
      default: {
        const handler = spec.commands?.[command];
        if (typeof handler === 'function') {
          const output = await handler({
            intentStore: this.intentStore,
            params: subscription.params,
            filter: subscription.filter,
            connection: connection.context,
            args
          });
          return { status: 'ok', channel, command, result: output };
        }
        return { status: 'error', code: 'UNKNOWN_COMMAND' };
      }
    }
  }

  broadcast(entries = [], { reason = 'intent-flush' } = {}) {
    const grouped = new Map();
    for (const entry of entries) {
      const channels = this._extractChannels(entry);
      for (const channelId of channels) {
        const spec = this.channels.get(channelId);
        const subscribers = this.channelSubscriptions.get(channelId);
        if (!spec || !subscribers || subscribers.size === 0) continue;
        for (const [connectionId, subscriptionKeys] of subscribers.entries()) {
          const connection = this.connections.get(connectionId);
          if (!connection) continue;
          for (const subscriptionKey of subscriptionKeys) {
            const subscription = connection.subscriptions.get(subscriptionKey);
            if (!subscription) continue;
            if (!this._shouldDeliver(entry, spec, subscription, connection)) continue;
            const key = `${connectionId}:${subscriptionKey}`;
            if (!grouped.has(key)) {
              grouped.set(key, {
                channel: channelId,
                connectionId,
                params: subscription.params,
                intents: []
              });
            }
            grouped.get(key).intents.push(entry);
          }
        }
      }
    }

    for (const { channel, connectionId, intents, params } of grouped.values()) {
      const cursor = this._cursorForEntries(intents);
      this._send(connectionId, {
        type: 'channel.invalidate',
        channel,
        reason,
        cursor,
        params,
        intents
      });
    }
  }

  listSubscriptions() {
    const result = [];
    for (const [connectionId, connection] of this.connections.entries()) {
      for (const details of connection.subscriptions.values()) {
        result.push({
          connectionId,
          channel: details.channel,
          params: details.params,
          subscribedAt: details.subscribedAt,
          filter: details.filter || null,
          connectionRole: connection.context?.role,
          site: connection.context?.site,
          user: connection.context?.user || null
        });
      }
    }
    return result;
  }

  async _loadSnapshot(spec, { channel, params, connection, cursor = 0, limit = 200, filter = null } = {}) {
    if (spec.load) {
      const output = await spec.load({ intentStore: this.intentStore, params, connection, cursor, limit, filter });
      if (output && typeof output === 'object' && !Array.isArray(output)) {
        const entries = Array.isArray(output.entries) ? output.entries : [];
        const resolvedCursor = Number.isFinite(output.cursor) ? output.cursor : this._cursorForEntries(entries);
        return { entries, cursor: resolvedCursor };
      }
      const entries = Array.isArray(output) ? output : [];
      return { entries, cursor: this._cursorForEntries(entries) };
    }
    const entries = this.intentStore.entriesAfter(cursor, { limit, channels: [channel] });
    return { entries, cursor: this._cursorForEntries(entries) };
  }

  _cursorForEntries(entries) {
    if (!Array.isArray(entries) || entries.length === 0) {
      return typeof this.intentStore.latestCursor === 'function' ? this.intentStore.latestCursor() : 0;
    }
    const last = entries[entries.length - 1];
    return last?.logSeq || last?.insertedAt || 0;
  }

  _getSubscription(connectionId, channelId, params = {}) {
    const connection = this.connections.get(connectionId);
    if (!connection) return null;
    const key = this._subscriptionKey(channelId, params);
    return connection.subscriptions.get(key) || null;
  }

  _defaultSnapshot(channelId) {
    if (!this.intentStore?.entriesSince) return [];
    const since = Date.now() - this.replayWindowMs;
    return this.intentStore.entriesSince(since).filter((entry) => {
      const channels = this._extractChannels(entry);
      return channels.includes(channelId);
    });
  }

  _extractChannels(entry = {}) {
    const channels = entry.meta?.channels;
    if (Array.isArray(channels) && channels.length) return channels;
    return ['global'];
  }

  _shouldDeliver(entry, spec, subscription, connection) {
    if (!spec?.filter) return true;
    try {
      return spec.filter(entry, {
        params: subscription.params,
        filter: subscription.filter,
        connection: connection.context
      }) !== false;
    } catch (error) {
      console.warn('[ChannelRegistry] filter handler failed', error);
      return true;
    }
  }

  _removeSubscription(connectionId, channelId, subscriptionKey, notify, closeInfo) {
    const connection = this.connections.get(connectionId);
    if (!connection || !connection.subscriptions.has(subscriptionKey)) {
      return { status: 'error', code: 'NOT_SUBSCRIBED' };
    }
    const subscription = connection.subscriptions.get(subscriptionKey);
    connection.subscriptions.delete(subscriptionKey);
    const channelMap = this.channelSubscriptions.get(channelId);
    if (channelMap) {
      const set = channelMap.get(connectionId);
      if (set) {
        set.delete(subscriptionKey);
        if (set.size === 0) {
          channelMap.delete(connectionId);
        }
      }
      if (channelMap.size === 0) {
        this.channelSubscriptions.delete(channelId);
      }
    }
    if (notify) {
      this._send(connectionId, {
        type: 'channel.unsubscribed',
        channel: channelId
      });
    }
    if (closeInfo) {
      this._sendClose(connectionId, channelId, closeInfo, subscription?.params);
    }
    return { status: 'ok', channel: channelId };
  }

  async _resendSubscription(connectionId, subscription, spec, reason = 'manual-resend') {
    const connection = this.connections.get(connectionId);
    if (!connection) return null;
    if (spec.access) {
      const allowed = await spec.access(subscription.params, { params: subscription.params, connection: connection.context });
      if (allowed === false) {
        this._removeSubscription(connectionId, subscription.channel, this._subscriptionKey(subscription.channel, subscription.params), true, {
          code: 'ACCESS_DENIED',
          reason: 'Access revoked during resend'
        });
        return null;
      }
    }
    const loader = spec.resend || spec.load;
    let snapshot;
    if (loader) {
      const output = await loader({
        intentStore: this.intentStore,
        params: subscription.params,
        filter: subscription.filter,
        connection: connection.context,
        reason
      });
      if (output && typeof output === 'object' && !Array.isArray(output)) {
        const entries = Array.isArray(output.entries) ? output.entries : [];
        const cursor = Number.isFinite(output.cursor) ? output.cursor : this._cursorForEntries(entries);
        snapshot = { entries, cursor };
      } else {
        const entries = Array.isArray(output) ? output : [];
        snapshot = { entries, cursor: this._cursorForEntries(entries) };
      }
    } else {
      snapshot = await this._loadSnapshot(spec, {
        channel: subscription.channel,
        params: subscription.params,
        connection: connection.context,
        filter: subscription.filter
      });
    }
    this._send(connectionId, {
      type: 'channel.replay',
      channel: subscription.channel,
      intents: snapshot.entries,
      cursor: snapshot.cursor,
      reason,
      params: subscription.params
    });
    return snapshot;
  }

  _sendClose(connectionId, channelId, closeInfo = {}, params = {}) {
    const payload = {
      type: 'channel.close',
      channel: channelId,
      code: closeInfo.code || 'CHANNEL_CLOSED',
      reason: closeInfo.reason || 'Channel closed',
      params,
      timestamp: Date.now()
    };
    if (closeInfo.meta) {
      payload.meta = closeInfo.meta;
    }
    this._send(connectionId, payload);
  }

  _subscriptionKey(channel, params = {}) {
    const serialized = this._stableStringify(params || {});
    return `${channel}:${serialized}`;
  }

  _stableStringify(value) {
    if (Array.isArray(value)) {
      return `[${value.map((item) => this._stableStringify(item)).join(',')}]`;
    }
    if (value && typeof value === 'object') {
      const ordered = {};
      for (const key of Object.keys(value).sort()) {
        ordered[key] = value[key];
      }
      return JSON.stringify(ordered);
    }
    return JSON.stringify(value);
  }

  _send(connectionId, payload) {
    const connection = this.connections.get(connectionId);
    if (!connection) return;
    try {
      connection.send(payload);
    } catch (error) {
      console.warn('[ChannelRegistry] Failed to send channel payload', error);
    }
  }
}
