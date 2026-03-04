import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';

export class UserStore {
  constructor({ filePath } = {}) {
    this.filePath = filePath || path.resolve(process.cwd(), 'data/users.json');
    this.users = new Map();
    this._load();
  }

  async create({ email, passwordHash, name = '', role = 'user', status = 'active' }) {
    const normalizedEmail = email.trim().toLowerCase();
    if (this.findByEmail(normalizedEmail)) {
      const error = new Error('Email already registered');
      error.status = 409;
      throw error;
    }

    const now = new Date().toISOString();
    const user = {
      id: randomUUID(),
      email: normalizedEmail,
      passwordHash,
      name,
      role,
      status,
      createdAt: now,
      updatedAt: now,
      lastLoginAt: null
    };

    this.users.set(user.id, user);
    this._persist();
    return user;
  }

  update(userId, updates = {}) {
    const user = this.users.get(userId);
    if (!user) return null;
    const updated = {
      ...user,
      ...updates,
      updatedAt: new Date().toISOString()
    };
    this.users.set(userId, updated);
    this._persist();
    return updated;
  }

  findByEmail(email) {
    const normalized = email?.trim().toLowerCase();
    if (!normalized) return null;
    for (const user of this.users.values()) {
      if (user.email === normalized) {
        return user;
      }
    }
    return null;
  }

  findById(id) {
    return this.users.get(id) || null;
  }

  list() {
    return Array.from(this.users.values());
  }

  _load() {
    try {
      const dir = path.dirname(this.filePath);
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }
      if (!fs.existsSync(this.filePath)) {
        fs.writeFileSync(this.filePath, JSON.stringify({ users: [] }, null, 2), 'utf8');
      }
      const raw = fs.readFileSync(this.filePath, 'utf8');
      if (!raw) return;
      const parsed = JSON.parse(raw);
      const items = Array.isArray(parsed)
        ? parsed
        : Array.isArray(parsed.users)
          ? parsed.users
          : [];
      for (const user of items) {
        if (user?.id) {
          this.users.set(user.id, user);
        }
      }
    } catch (error) {
      console.error('[UserStore] Failed to load users:', error);
    }
  }

  _persist() {
    try {
      const payload = JSON.stringify({ users: this.list() }, null, 2);
      fs.writeFileSync(this.filePath, payload, 'utf8');
    } catch (error) {
      console.error('[UserStore] Failed to persist users:', error);
    }
  }
}
