import { validateContract } from '../runtime/ContractRegistry.js';
import { expectJsonBody } from '../utils/http.js';
import { rateLimiter } from '../middleware/rateLimiter.js';

export function registerLogRoutes(kernel, logService, config) {
  kernel.route('POST', '/logs/batch', async (ctx) => {
    const limiter = rateLimiter({
      name: 'logs:batch',
      windowMs: 60000,
      max: 60
    });

    await limiter(ctx, async () => {
      const payload = expectJsonBody(ctx);
      validateContract('logs', 'INTENT_LOG_BATCH', payload);

      const result = await logService.ingestBatch(payload);
      ctx.json(202, result);
    });
  });

  kernel.route('GET', '/logs/health', async (ctx) => {
    ctx.json(200, logService.health());
  });
}
