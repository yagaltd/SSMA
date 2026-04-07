import { appendFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';

export class LogService {
  constructor(config = {}) {
    this.bufferSize = config.bufferSize || 100;
    this.maxBatchSize = config.maxBatchSize || 50;
    this.filePath = config.filePath || './logs/ssma.log';
    this.exporter = config.exporter || 'file';
    this.flushIntervalMs = config.flushIntervalMs || 15000;
    
    this.buffer = [];
    this.metrics = {
      ingested: 0,
      exported: 0,
      errors: 0
    };
    
    this.initializeLogFile();
    this.startPeriodicFlush();
  }

  initializeLogFile() {
    const dir = dirname(this.filePath);
    if (!existsSync(dir)) {
      mkdirSync(dir, { recursive: true });
    }
  }

  async ingestBatch(payload) {
    try {
      const { entries, userId, sessionId } = payload;
      const events = Array.isArray(entries) ? entries : [entries];
      
      const normalized = events.map(event => ({
        timestamp: event.timestamp || Date.now(),
        type: event.type || 'log',
        level: event.level || 'info',
        data: event.data || event,
        userId: userId,
        sessionId: sessionId
      }));

      this.buffer.push(...normalized);
      this.metrics.ingested += normalized.length;

      if (this.buffer.length >= this.maxBatchSize) {
        await this.flush();
      }

      return {
        success: true,
        accepted: normalized.length,
        bufferSize: this.buffer.length
      };
    } catch (error) {
      this.metrics.errors++;
      throw error;
    }
  }

  async ingestServerEvent(type, data = {}, level = 'info') {
    return this.ingestBatch({
      entries: [{ type, level, data, timestamp: Date.now() }],
      userId: data?.userId,
      sessionId: data?.sessionId
    }).catch((error) => {
      console.warn('[LogService] Failed to ingest server event', type, error);
    });
  }

  async flush() {
    if (this.buffer.length === 0) return;

    const batch = [...this.buffer];
    this.buffer = [];

    try {
      switch (this.exporter) {
        case 'file':
          this.exportToFile(batch);
          break;
        case 'console':
          batch.forEach(event => console.log(JSON.stringify(event)));
          break;
        default:
          console.warn(`Unknown exporter: ${this.exporter}, using console`);
          batch.forEach(event => console.log(JSON.stringify(event)));
      }
      this.metrics.exported += batch.length;
    } catch (error) {
      this.metrics.errors++;
      this.buffer.unshift(...batch);
      throw error;
    }
  }

  exportToFile(batch) {
    const lines = batch.map(event => JSON.stringify(event)).join('\n') + '\n';
    appendFileSync(this.filePath, lines, 'utf8');
  }

  startPeriodicFlush() {
    setInterval(async () => {
      if (this.buffer.length > 0) {
        await this.flush().catch(err => {
          console.error('Periodic flush failed:', err);
        });
      }
    }, this.flushIntervalMs);
  }

  health() {
    return {
      status: this.metrics.errors > 10 ? 'degraded' : 'ok',
      bufferSize: this.buffer.length,
      metrics: { ...this.metrics },
      exporter: this.exporter,
      filePath: this.filePath
    };
  }
}
