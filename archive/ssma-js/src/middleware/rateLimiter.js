const buckets = new Map();

export function rateLimiter({
  windowMs = 60000,
  max = 100,
  name = 'default',
  keyResolver,
  shouldApply,
  onLimit
} = {}) {
  return async (ctx, next) => {
    if (typeof shouldApply === 'function' && !shouldApply(ctx)) {
      await next();
      return;
    }

    const resolvedKey = typeof keyResolver === 'function'
      ? keyResolver(ctx)
      : (ctx.ip || 'unknown');
    const key = `${name}:${resolvedKey}`;
    const now = Date.now();
    const bucket = buckets.get(key) || { count: 0, expiresAt: now + windowMs };

    if (bucket.expiresAt < now) {
      bucket.count = 0;
      bucket.expiresAt = now + windowMs;
    }

    bucket.count += 1;
    bucket.name = name;
    bucket.windowMs = windowMs;
    bucket.max = max;
    buckets.set(key, bucket);

    if (bucket.count > max) {
      const retryAfterMs = bucket.expiresAt - now;
      ctx.json(429, { error: 'Too many requests', retryAfterMs });
      if (typeof onLimit === 'function') {
        await onLimit(ctx, { ...bucket, retryAfterMs });
      }
      return;
    }

    await next();
  };
}
