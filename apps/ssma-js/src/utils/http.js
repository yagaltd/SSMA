export function expectJsonBody(ctx) {
  if (ctx.body && typeof ctx.body === 'object' && !Array.isArray(ctx.body)) {
    return ctx.body;
  }
  if (!ctx.body) {
    return {};
  }
  const error = new Error('Request body must be JSON object');
  error.status = 400;
  throw error;
}
