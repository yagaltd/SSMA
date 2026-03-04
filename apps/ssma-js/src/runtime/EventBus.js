import { validateContract } from './ContractRegistry.js';

export class EventBus {
  constructor(logger = console) {
    this.logger = logger;
    this.handlers = new Map();
  }

  on(eventName, handler) {
    if (!this.handlers.has(eventName)) {
      this.handlers.set(eventName, new Set());
    }
    this.handlers.get(eventName).add(handler);
  }

  off(eventName, handler) {
    this.handlers.get(eventName)?.delete(handler);
  }

  async emit(contractGroup, eventName, payload) {
    validateContract(contractGroup, eventName, payload);

    const listeners = Array.from(this.handlers.get(eventName) || []);
    for (const listener of listeners) {
      try {
        await listener(payload);
      } catch (error) {
        this.logger.error(`[EventBus] listener error for ${eventName}`, error);
      }
    }

    this.logger.info(`[EventBus] ${eventName}`, payload);
  }
}
