import { hasRole } from '../utils/rbac.js';

export function registerIslandRoutes(kernel, islandService) {
  if (!islandService) {
    return;
  }

  kernel.route('GET', '/islands/inventory', (ctx) => {
    ctx.json(200, {
      updatedAt: Date.now(),
      products: islandService.listInventory()
    });
  });

  kernel.route('POST', '/islands/inventory/:productId', (ctx) => {
    const productId = ctx.params.productId;
    const quantity = Number(ctx.body?.quantity);
    if (!Number.isFinite(quantity) || quantity < 0) {
      ctx.json(400, { error: 'INVALID_QUANTITY', message: 'Quantity must be a non-negative number' });
      return;
    }

    const record = islandService.updateInventory(productId, quantity, {
      userId: ctx.state.user?.id || 'anonymous',
      source: 'api',
      reason: ctx.body?.reason || 'manual-update'
    });

    ctx.json(200, { product: record });
  });

  kernel.route('GET', '/islands/reviews/:productId', (ctx) => {
    ctx.json(200, {
      updatedAt: Date.now(),
      reviews: islandService.listReviews(ctx.params.productId)
    });
  });

  kernel.route('POST', '/islands/reviews/:productId', (ctx) => {
    if (!hasRole(ctx.state.user?.role, 'user')) {
      ctx.json(403, { error: 'FORBIDDEN', requiredRole: 'user' });
      return;
    }
    const entry = islandService.recordReview(ctx.params.productId, {
      rating: ctx.body?.rating,
      headline: ctx.body?.headline,
      body: ctx.body?.body,
      author: ctx.state.user?.id || 'anonymous',
      userId: ctx.state.user?.id,
      reason: ctx.body?.reason || 'review-api'
    });
    ctx.json(201, { review: entry });
  });

  kernel.route('GET', '/islands/blog/:slug/comments', (ctx) => {
    ctx.json(200, {
      updatedAt: Date.now(),
      comments: islandService.listComments(ctx.params.slug)
    });
  });

  kernel.route('POST', '/islands/blog/:slug/comments', (ctx) => {
    if (!hasRole(ctx.state.user?.role, 'user')) {
      ctx.json(403, { error: 'FORBIDDEN', requiredRole: 'user' });
      return;
    }
    const comment = islandService.addComment(ctx.params.slug, {
      body: ctx.body?.body,
      author: ctx.state.user?.id || 'anonymous',
      userId: ctx.state.user?.id,
      reason: ctx.body?.reason || 'comment-api'
    });
    ctx.json(201, { comment });
  });

  kernel.route('POST', '/islands/invalidate', (ctx) => {
    if (!hasRole(ctx.state.user?.role, 'staff')) {
      ctx.json(403, { error: 'FORBIDDEN', requiredRole: 'staff' });
      return;
    }
    const payload = islandService.publishInvalidation({
      islandId: ctx.body?.islandId,
      parameters: ctx.body?.parameters || {},
      reason: ctx.body?.reason || 'manual-api',
      dataContract: ctx.body?.dataContract,
      payload: ctx.body?.payload || {}
    });
    ctx.json(202, { status: 'scheduled', event: payload });
  });
}
