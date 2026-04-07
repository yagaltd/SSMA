import { describe, it, expect, vi } from 'vitest';
import { IslandInvalidationService } from '../src/services/island-invalidations/IslandInvalidationService.js';

describe('IslandInvalidationService', () => {
  it('publishes invalidations when inventory changes', () => {
    const publish = vi.fn();
    const broadcast = vi.fn();
    const ingest = vi.fn();

    const service = new IslandInvalidationService({
      eventHub: { publish },
      syncGateway: { broadcast },
      logService: { ingestServerEvent: ingest }
    });

    const record = service.updateInventory('SKU-TEST', 42, { reason: 'unit-test', userId: 'bot' });

    expect(record.productId).toBe('SKU-TEST');
    expect(record.quantity).toBe(42);
    expect(publish).toHaveBeenCalledTimes(1);
    expect(broadcast).toHaveBeenCalledTimes(1);
    const payload = publish.mock.calls[0][1];
    expect(payload).toMatchObject({
      islandId: 'product-inventory',
      parameters: { productId: 'SKU-TEST' },
      reason: 'unit-test'
    });
  });

  it('can disable invalidations via feature flags', () => {
    const publish = vi.fn();
    const broadcast = vi.fn();
    const service = new IslandInvalidationService({
      eventHub: { publish },
      syncGateway: { broadcast },
      featureFlags: { enabled: false }
    });
    const result = service.publishInvalidation({ islandId: 'product-inventory', parameters: {} });
    expect(result.skipped).toBe(true);
    expect(publish).not.toHaveBeenCalled();
    expect(broadcast).not.toHaveBeenCalled();
  });

  it('records product reviews and emits invalidations', () => {
    const publish = vi.fn();
    const service = new IslandInvalidationService({
      eventHub: { publish },
      syncGateway: { broadcast: vi.fn() }
    });

    const review = service.recordReview('SKU-002', {
      rating: 4,
      headline: 'Solid pack',
      body: 'Great hybrid performance',
      reason: 'unit-test'
    });

    expect(review.productId).toBe('SKU-002');
    const payload = publish.mock.calls.at(-1)[1];
    expect(payload.islandId).toBe('product-reviews');
    expect(payload.parameters).toEqual({ productId: 'SKU-002' });
    expect(payload.dataContract).toBe('PRODUCT_REVIEWS_REQUESTED');
  });

  it('records blog comments and notifies subscribers', () => {
    const publish = vi.fn();
    const broadcast = vi.fn();
    const service = new IslandInvalidationService({
      eventHub: { publish },
      syncGateway: { broadcast }
    });

    const comment = service.addComment('hybrid-testing', {
      body: 'New comment',
      author: 'pat'
    });

    expect(comment.postId).toBe('hybrid-testing');
    const payload = publish.mock.calls.at(-1)[1];
    expect(payload.islandId).toBe('blog-comments');
    expect(payload.parameters).toEqual({ postId: 'hybrid-testing' });
    expect(payload.dataContract).toBe('COMMENT_LIST_REQUESTED');
    expect(broadcast).toHaveBeenCalled();
  });
});
