export class HybridMonitor {
  constructor({ logService, backlogThreshold = 1000, invalidationLatencyBudgetMs = 5000 } = {}) {
    this.logService = logService;
    this.backlogThreshold = backlogThreshold;
    this.latencyBudgetMs = invalidationLatencyBudgetMs;
    this.metrics = {
      backlogDepth: 0,
      peakBacklog: 0,
      backlogBreaches: 0,
      latency: {
        sse: { count: 0, sum: 0, max: 0, avg: 0 },
        ws: { count: 0, sum: 0, max: 0, avg: 0 }
      },
      lastViolationAt: null
    };
  }

  recordBacklog(count = 0) {
    this.metrics.backlogDepth = count;
    this.metrics.peakBacklog = Math.max(this.metrics.peakBacklog, count);
    if (count > this.backlogThreshold) {
      this.metrics.backlogBreaches += 1;
      this.metrics.lastViolationAt = Date.now();
      this.logService?.ingestServerEvent?.('HYBRID_BACKLOG_THRESHOLD_EXCEEDED', {
        backlog: count,
        threshold: this.backlogThreshold
      }, 'warn');
    }
  }

  recordInvalidationLatency(channel, latencyMs) {
    if (!channel || !Number.isFinite(latencyMs)) {
      return;
    }
    const bucket = this.metrics.latency[channel] || (this.metrics.latency[channel] = { count: 0, sum: 0, max: 0, avg: 0 });
    bucket.count += 1;
    bucket.sum += latencyMs;
    bucket.max = Math.max(bucket.max, latencyMs);
    bucket.avg = bucket.sum / bucket.count;
    if (this.latencyBudgetMs && latencyMs > this.latencyBudgetMs) {
      this.metrics.lastViolationAt = Date.now();
      this.logService?.ingestServerEvent?.('HYBRID_INVALIDATION_LATENCY_BUDGET_EXCEEDED', {
        channel,
        latencyMs,
        budget: this.latencyBudgetMs
      }, 'warn');
    }
  }

  snapshot() {
    return JSON.parse(JSON.stringify({
      ...this.metrics,
      latencyBudgetMs: this.latencyBudgetMs,
      backlogThreshold: this.backlogThreshold
    }));
  }
}
