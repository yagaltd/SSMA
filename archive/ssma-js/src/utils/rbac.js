const ROLE_ORDER = ['guest', 'user', 'staff', 'admin', 'system'];

export const ROLE_MATRIX = {
  guest: ['read:public'],
  user: ['read:public', 'read:self', 'write:self'],
  staff: ['read:public', 'read:self', 'write:self', 'moderate:tenant'],
  admin: ['*'],
  system: ['*']
};

export function hasRole(userRole, requiredRole) {
  const userIdx = ROLE_ORDER.indexOf(userRole || 'guest');
  const requiredIdx = ROLE_ORDER.indexOf(requiredRole);
  if (requiredIdx === -1) return false;
  return userIdx >= requiredIdx;
}

export function requireRole(requiredRole) {
  return async (ctx, next) => {
    const currentRole = ctx.state.user?.role || 'guest';
    if (!hasRole(currentRole, requiredRole)) {
      ctx.json(403, { error: 'FORBIDDEN', requiredRole });
      return;
    }
    await next();
  };
}
