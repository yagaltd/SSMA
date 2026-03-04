import { serialize } from 'cookie';

export function registerAuthRoutes(kernel, authService, config) {
  const cookieName = config.auth?.cookieName || 'ssma_session';
  const cookieOptions = {
    httpOnly: true,
    secure: config.auth?.cookieSecure ?? true,
    sameSite: 'lax',
    path: '/',
    maxAge: 60 * 60 // 1 hour default, matches 15m token auto-rotation but keeps cookie alive
  };

  const setSessionCookie = (ctx, token) => {
    ctx.res.setHeader('Set-Cookie', serialize(cookieName, token, cookieOptions));
  };

  const clearSessionCookie = (ctx) => {
    ctx.res.setHeader(
      'Set-Cookie',
      serialize(cookieName, '', {
        ...cookieOptions,
        maxAge: 0
      })
    );
  };

  kernel.route('POST', '/auth/register', async (ctx) => {
    const { name, email, password } = ctx.body || {};
    if (!password || password.length < 8) {
      ctx.json(400, { error: 'INVALID_PASSWORD', message: 'Password must be at least 8 characters' });
      return;
    }
    const result = await authService.register({ name, email, password });
    const token = await authService.issueToken({ ...result.user });
    setSessionCookie(ctx, token);
    ctx.json(201, result);
  });

  kernel.route('POST', '/auth/login', async (ctx) => {
    const { email, password } = ctx.body || {};
    if (!email || !password) {
      ctx.json(400, { error: 'MISSING_CREDENTIALS' });
      return;
    }
    const result = await authService.login({ email, password });
    setSessionCookie(ctx, result.token);
    ctx.json(200, { user: result.user });
  });

  kernel.route('POST', '/auth/logout', async (ctx) => {
    clearSessionCookie(ctx);
    ctx.json(200, { success: true });
  });

  kernel.route('GET', '/auth/me', async (ctx) => {
    if (!ctx.state.user) {
      ctx.json(401, { error: 'UNAUTHENTICATED' });
      return;
    }
    const user = await authService.getUser(ctx.state.user.id);
    if (!user) {
      ctx.json(404, { error: 'USER_NOT_FOUND' });
      return;
    }
    ctx.json(200, { user });
  });
}
