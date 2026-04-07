export function corsMiddleware({ origins = ['*'] } = {}) {
  const allowAll = origins.includes('*');

  return (ctx, next) => {
    const requestOrigin = ctx.headers.origin;
    if (allowAll || (requestOrigin && origins.includes(requestOrigin))) {
      ctx.res.setHeader('Access-Control-Allow-Origin', requestOrigin || '*');
      ctx.res.setHeader('Vary', 'Origin');
    }
    ctx.res.setHeader('Access-Control-Allow-Credentials', 'true');
    ctx.res.setHeader(
      'Access-Control-Allow-Headers',
      ctx.headers['access-control-request-headers'] || 'Content-Type, Authorization'
    );
    ctx.res.setHeader('Access-Control-Allow-Methods', 'GET,POST,PUT,PATCH,DELETE,OPTIONS');

    if (ctx.method === 'OPTIONS') {
      ctx.res.statusCode = 204;
      ctx.res.end();
      ctx.responded = true;
      return Promise.resolve();
    }

    return next();
  };
}
