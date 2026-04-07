import { describe, it, expect, beforeEach } from 'vitest';
import os from 'node:os';
import path from 'node:path';
import fs from 'node:fs';
import { AuthService } from '../src/services/auth/AuthService.js';
import { UserStore } from '../src/services/auth/UserStore.js';

describe('AuthService', () => {
  let storePath;
  let authService;

  beforeEach(() => {
    storePath = path.join(os.tmpdir(), `ssma-users-${Date.now()}-${Math.random()}.json`);
    if (fs.existsSync(storePath)) {
      fs.unlinkSync(storePath);
    }
    authService = new AuthService({
      userStore: new UserStore({ filePath: storePath }),
      jwtSecret: 'test-secret',
      jwtExpiresIn: '5m'
    });
  });

  it('registers and logs in a user', async () => {
    const register = await authService.register({
      email: 'user@example.com',
      password: 's3curep@ss',
      name: 'Demo User'
    });
    expect(register.user.email).toBe('user@example.com');
    expect(register.user.role).toBe('user');

    const login = await authService.login({
      email: 'user@example.com',
      password: 's3curep@ss'
    });
    expect(login.user.id).toBe(register.user.id);
    expect(login.token).toBeTruthy();

    const verified = await authService.verifyToken(login.token);
    expect(verified.sub).toBe(register.user.id);
    expect(verified.role).toBe('user');
  });
});
