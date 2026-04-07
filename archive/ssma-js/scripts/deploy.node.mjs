#!/usr/bin/env node
import { spawnSync } from 'node:child_process';

const steps = ['sync:contracts', 'check:security', 'generate:map'];

for (const step of steps) {
  const result = spawnSync('npm', ['run', step], { stdio: 'inherit' });
  if (result.status !== 0) {
    console.error(`[deploy.node] step ${step} failed`);
    process.exit(result.status || 1);
  }
}

console.log('[deploy.node] ready for Node/Bun deployment. Bundle dist via your platform tooling.');
