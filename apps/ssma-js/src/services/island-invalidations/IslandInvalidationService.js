import { randomUUID } from 'node:crypto';

export class IslandInvalidationService {
  constructor({ eventHub, syncGateway, logService, featureFlags = {} } = {}) {
    this.eventHub = eventHub;
    this.syncGateway = syncGateway;
    this.logService = logService;
    this.featureFlags = featureFlags;
    this.inventory = new Map([
      ['SKU-001', { productId: 'SKU-001', quantity: 12, title: 'Heritage Backpack', updatedAt: Date.now() }],
      ['SKU-002', { productId: 'SKU-002', quantity: 7, title: 'Trailfinder Boots', updatedAt: Date.now() }],
      ['SKU-003', { productId: 'SKU-003', quantity: 3, title: 'Summit Shell', updatedAt: Date.now() }]
    ]);
    this.reviews = new Map([
      ['SKU-001', [
        {
          id: randomUUID(),
          productId: 'SKU-001',
          rating: 5,
          headline: 'Trail Ready',
          body: 'Took it on a 5-day hike and the stitching held up.',
          author: 'alex',
          createdAt: Date.now() - 1000
        }
      ]]
    ]);
    this.comments = new Map([
      ['hybrid-testing', [
        {
          id: randomUUID(),
          postId: 'hybrid-testing',
          body: 'Loving the hybrid approach so far!',
          author: 'casey',
          createdAt: Date.now() - 2000
        }
      ]]
    ]);
  }

  listInventory() {
    return Array.from(this.inventory.values()).map((record) => ({ ...record }));
  }

  getInventory(productId) {
    const record = this.inventory.get(productId);
    return record ? { ...record } : null;
  }

  updateInventory(productId, quantity, options = {}) {
    if (!productId) {
      throw new Error('PRODUCT_ID_REQUIRED');
    }
    if (!Number.isFinite(quantity)) {
      throw new Error('INVALID_QUANTITY');
    }

    const record = {
      productId,
      quantity,
      title: options.title || this.inventory.get(productId)?.title || 'Unknown Product',
      updatedAt: Date.now()
    };
    this.inventory.set(productId, record);

    this.logService?.ingestServerEvent?.('INVENTORY_UPDATED', {
      productId,
      quantity,
      userId: options.userId || 'system',
      source: options.source || 'api'
    });

    this.publishInvalidation({
      islandId: 'product-inventory',
      parameters: { productId },
      dataContract: 'INVENTORY_INQUIRY',
      reason: options.reason || 'inventory-update',
      payload: record
    });

    return { ...record };
  }

  listReviews(productId) {
    return [...(this.reviews.get(productId) || [])];
  }

  recordReview(productId, review = {}) {
    if (!productId) {
      throw new Error('PRODUCT_ID_REQUIRED');
    }
    const entry = {
      id: review.id || randomUUID(),
      productId,
      rating: Number.isFinite(review.rating) ? Number(review.rating) : 5,
      headline: review.headline || 'New review',
      body: review.body || 'Updated sentiment from hybrid pilot.',
      author: review.author || 'anonymous',
      createdAt: Date.now()
    };
    const reviews = this.reviews.get(productId) || [];
    reviews.unshift(entry);
    this.reviews.set(productId, reviews.slice(0, 25));
    this.logService?.ingestServerEvent?.('PRODUCT_REVIEW_RECORDED', {
      productId,
      rating: entry.rating,
      headline: entry.headline,
      userId: review.userId || 'anonymous'
    });
    this.publishInvalidation({
      islandId: 'product-reviews',
      parameters: { productId },
      dataContract: 'PRODUCT_REVIEWS_REQUESTED',
      reason: review.reason || 'review-update',
      payload: { reviews: this.listReviews(productId).slice(0, 5) }
    });
    return entry;
  }

  listComments(postId) {
    return [...(this.comments.get(postId) || [])];
  }

  addComment(postId, comment = {}) {
    if (!postId) {
      throw new Error('POST_ID_REQUIRED');
    }
    const entry = {
      id: comment.id || randomUUID(),
      postId,
      body: comment.body || 'New hybrid comment',
      author: comment.author || 'anonymous',
      createdAt: Date.now()
    };
    const comments = this.comments.get(postId) || [];
    comments.unshift(entry);
    this.comments.set(postId, comments.slice(0, 50));
    this.logService?.ingestServerEvent?.('BLOG_COMMENT_ADDED', {
      postId,
      author: entry.author,
      userId: comment.userId || 'anonymous'
    });
    this.publishInvalidation({
      islandId: 'blog-comments',
      parameters: { postId },
      dataContract: 'COMMENT_LIST_REQUESTED',
      reason: comment.reason || 'comment-created',
      payload: { comments: this.listComments(postId).slice(0, 10) }
    });
    return entry;
  }

  publishInvalidation({ islandId, parameters = {}, reason = 'manual', dataContract, payload = {} } = {}) {
    if (!islandId) {
      throw new Error('ISLAND_ID_REQUIRED');
    }
    const body = {
      eventId: randomUUID(),
      islandId,
      parameters,
      reason,
      site: 'default',
      cursor: Date.now(),
      timestamp: Date.now(),
      dataContract,
      payload
    };

    if (!this.#canInvalidate(islandId)) {
      this.logService?.ingestServerEvent?.('ISLAND_INVALIDATION_SKIPPED', {
        islandId,
        reason: 'feature-disabled'
      }, 'warn');
      return { skipped: true, reason: 'feature-disabled', islandId };
    }

    this.eventHub?.publish('island.invalidate', body);
    this.syncGateway?.broadcast('island.invalidate', body);
    this.logService?.ingestServerEvent?.('ISLAND_INVALIDATED', body);
    return body;
  }

  #canInvalidate(islandId) {
    if (this.featureFlags?.enabled === false) {
      return false;
    }
    const disabled = this.featureFlags?.disabledIslands || [];
    if (disabled.length && disabled.includes(islandId)) {
      return false;
    }
    return true;
  }
}
