#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import dotenv from 'dotenv';

const projectRoot = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const envPath = path.join(projectRoot, '.env');

const result = {
  issues: [],
  warnings: []
};

if (!fs.existsSync(envPath)) {
  result.issues.push('.env file missing. Copy .env.example and set secrets.');
} else {
  const env = dotenv.parse(fs.readFileSync(envPath));
  enforceSecret(env, 'SSMA_JWT_SECRET', 32);
  enforceSecret(env, 'SSMA_HMAC_SECRET', 32);
  enforceTTL(env, 'SSMA_ACCESS_TTL_MS', 60000, 3600000);
  enforceTTL(env, 'SSMA_REFRESH_TTL_MS', 600000, 1209600000);
  enforceTTL(env, 'SSMA_RATE_WINDOW_MS', 1000, 3600000);
  enforceNumber(env, 'SSMA_RATE_MAX', 10, 1000);
  enforceOrigins(env.SSMA_ALLOWED_ORIGINS);
}

const pkgPath = path.join(projectRoot, 'package.json');
const pkg = JSON.parse(fs.readFileSync(pkgPath, 'utf-8'));
if (!pkg.dependencies?.argon2) {
  result.warnings.push('argon2 dependency missing, password hashing will be insecure.');
}

if (result.issues.length > 0) {
  console.error('[check-security] FAILED');
  result.issues.forEach((issue) => console.error(` - ${issue}`));
  process.exit(1);
}

console.log('[check-security] passed');
if (result.warnings.length) {
  result.warnings.forEach((warning) => console.warn(`warning: ${warning}`));
}

function enforceSecret(env, key, minLength) {
  const value = env[key];
  if (!value || value.length < minLength || /change|secret|demo/i.test(value)) {
    result.issues.push(`${key} must be at least ${minLength} chars and unique.`);
  }
}

function enforceTTL(env, key, min, max) {
  const value = Number(env[key]);
  if (!Number.isFinite(value) || value < min || value > max) {
    result.issues.push(`${key} must be between ${min} and ${max} ms.`);
  }
}

function enforceNumber(env, key, min, max) {
  const value = Number(env[key]);
  if (!Number.isFinite(value) || value < min || value > max) {
    result.issues.push(`${key} must be between ${min} and ${max}.`);
  }
}

function enforceOrigins(origins) {
  if (!origins) return;
  const list = origins.split(',').map((o) => o.trim());
  for (const origin of list) {
    if (origin === '*' || origin === 'http://localhost' || origin.startsWith('http://localhost:')) continue;
    if (!origin.startsWith('https://')) {
      result.warnings.push(`Origin ${origin} is not HTTPS.`);
    }
  }
}
