export function requestLogger() {
  return async (ctx, next) => {
    const start = Date.now();
    try {
      await next();
    } finally {
      if (ctx.responded) {
        const duration = Date.now() - start;
        console.log(`${ctx.method} ${ctx.path} ${ctx.res.statusCode} ${duration}ms`);
      }
    }
  };
}
