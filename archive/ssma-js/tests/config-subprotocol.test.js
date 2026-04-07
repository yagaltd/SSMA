import { describe, it, expect, vi, afterEach } from 'vitest';
import { loadConfig } from '../src/config/env.js';

describe('subprotocol env resolution', () => {
  const original = { ...process.env };

  afterEach(() => {
    process.env = { ...original };
    vi.restoreAllMocks();
  });

  it('prefers SSMA_PROTOCOL_SUBPROTOCOL over legacy alias', () => {
    process.env.SSMA_PROTOCOL_SUBPROTOCOL = '1.2.3';
    process.env.SSMA_OPTIMISTIC_SUBPROTOCOL = '1.0.0';
    const config = loadConfig();
    expect(config.optimistic.subprotocol).toBe('1.2.3');
  });

  it('falls back to SSMA_OPTIMISTIC_SUBPROTOCOL when canonical is unset', () => {
    delete process.env.SSMA_PROTOCOL_SUBPROTOCOL;
    process.env.SSMA_OPTIMISTIC_SUBPROTOCOL = '1.9.9';
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
    const config = loadConfig();
    expect(config.optimistic.subprotocol).toBe('1.9.9');
    expect(warnSpy).toHaveBeenCalled();
  });

  it('defaults to 1.0.0 when both env vars are unset', () => {
    delete process.env.SSMA_PROTOCOL_SUBPROTOCOL;
    delete process.env.SSMA_OPTIMISTIC_SUBPROTOCOL;
    const config = loadConfig();
    expect(config.optimistic.subprotocol).toBe('1.0.0');
  });
});
