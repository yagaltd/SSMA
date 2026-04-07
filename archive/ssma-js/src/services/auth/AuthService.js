import argon2 from 'argon2';
import { SignJWT, jwtVerify } from 'jose';
import { TextEncoder } from 'node:util';

const DEFAULT_JWT_EXPIRES = '15m';
const TOKEN_HEADER = { alg: 'HS256', typ: 'JWT' };
const textEncoder = new TextEncoder();
const ARGON_OPTIONS = {
  type: argon2.argon2id,
  memoryCost: 48 * 1024,
  timeCost: 3,
  parallelism: 1
};

export class AuthService {
  constructor({ userStore, jwtSecret, jwtExpiresIn = DEFAULT_JWT_EXPIRES }) {
    if (!userStore) throw new Error('[AuthService] userStore is required');
    if (!jwtSecret) throw new Error('[AuthService] jwtSecret is required');
    this.userStore = userStore;
    this.jwtExpiresIn = jwtExpiresIn;
    this.jwtKey = textEncoder.encode(jwtSecret);
  }

  async register({ email, password, name = '', role = 'user' }) {
    if (!email || !password) {
      const error = new Error('Email and password are required');
      error.status = 400;
      throw error;
    }
    const passwordHash = await argon2.hash(password, ARGON_OPTIONS);
    const user = await this.userStore.create({ email, passwordHash, name, role });
    return { user: this.sanitizeUser(user) };
  }

  async login({ email, password }) {
    const user = this.userStore.findByEmail(email || '');
    if (!user || user.status !== 'active') {
      const error = new Error('Invalid credentials');
      error.status = 401;
      throw error;
    }
    const match = await argon2.verify(user.passwordHash, password || '');
    if (!match) {
      const error = new Error('Invalid credentials');
      error.status = 401;
      throw error;
    }
    const updated = this.userStore.update(user.id, { lastLoginAt: new Date().toISOString() });
    return {
      user: this.sanitizeUser(updated),
      token: await this.issueToken(updated)
    };
  }

  async issueToken(user) {
    return await new SignJWT({ role: user.role })
      .setProtectedHeader(TOKEN_HEADER)
      .setSubject(user.id)
      .setIssuedAt()
      .setExpirationTime(this.jwtExpiresIn)
      .sign(this.jwtKey);
  }

  async verifyToken(token) {
    try {
      const { payload } = await jwtVerify(token, this.jwtKey);
      return payload;
    } catch (error) {
      return null;
    }
  }

  async getUser(userId) {
    const user = this.userStore.findById(userId);
    return user ? this.sanitizeUser(user) : null;
  }

  sanitizeUser(user) {
    if (!user) return null;
    const { passwordHash, ...safe } = user;
    return safe;
  }
}
