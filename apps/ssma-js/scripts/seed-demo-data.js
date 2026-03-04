#!/usr/bin/env node
import { loadConfig } from '../src/config/env.js';
import { AuthService } from '../src/services/auth/AuthService.js';
import { EventBus } from '../src/runtime/EventBus.js';
import { userStore } from '../src/storage/userStore.js';
import { HmacService } from '../src/services/hmacService.js';

async function main() {
  const config = loadConfig();
  const eventBus = new EventBus({ info: () => {}, error: console.error });
  const hmacService = new HmacService(config);
  const service = new AuthService(config, eventBus, hmacService);

  const email = 'demo@ssma.local';
  let user = await userStore.findByEmail(email);
  if (!user) {
    ({ user } = await service.register({ email, password: 'demo-password', name: 'Demo User' }));
  }

  const apiKey = await service.seedApiKey(email);
  console.log('[seed-demo-data] demo user ready:', user.email);
  console.log('[seed-demo-data] apiKey:', apiKey.apiKey);
}

main().catch((error) => {
  console.error('[seed-demo-data] failed', error);
  process.exit(1);
});
