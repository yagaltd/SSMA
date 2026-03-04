import { parse } from 'cookie';

export function authMiddleware(authService, config = {}) {
  const cookieName = config.auth?.cookieName || 'ssma_session';
  return async (ctx, next) => {
    ctx.state = ctx.state || {};
    try {
      const cookies = parse(ctx.headers.cookie || '');
      const token = cookies[cookieName];
      if (token) {
        const payload = await authService.verifyToken(token);
        if (payload) {
          ctx.state.user = {
            id: payload.sub,
            role: payload.role || 'user'
          };
        }
      }
    } catch (error) {
      console.warn('[AuthMiddleware] Failed to parse auth cookie:', error);
    }

    await next();
  };
}
