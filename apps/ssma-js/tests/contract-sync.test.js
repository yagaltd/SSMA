import { describe, it, expect, afterEach } from 'vitest';
import { mkdtempSync, rmSync, writeFileSync, readFileSync, mkdirSync } from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { spawnSync } from 'node:child_process';

const SCRIPT_PATH = path.resolve(__dirname, '../scripts/sync-contracts.mjs');

describe('sync-contracts script', () => {
  const tmpDirs = [];

  afterEach(() => {
    while (tmpDirs.length) {
      const dir = tmpDirs.pop();
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('mirrors contracts and emits declarations', () => {
    const { sourceDir, targetDir, root } = createFixture();
    const contract = {
      DEMO_EVENT: {
        version: 1,
        type: 'event',
        owner: 'demo',
        schema: { type: 'object', properties: { foo: { type: 'string' } } }
      }
    };
    writeFileSync(path.join(sourceDir, 'demo.json'), JSON.stringify(contract, null, 2));

    const result = runScript(['--source', sourceDir, '--target', targetDir], root);
    expect(result.status).toBe(0);
    expect(readFileSync(path.join(targetDir, 'demo.json'), 'utf-8')).toContain('DEMO_EVENT');
    expect(readFileSync(path.join(targetDir, 'demo.js'), 'utf-8')).toContain('export const demoContracts');
    expect(readFileSync(path.join(targetDir, 'demo.d.ts'), 'utf-8')).toContain("'DEMO_EVENT'");
  });

  it('fails in check mode when artifacts are stale', () => {
    const { sourceDir, targetDir, root } = createFixture();
    const contract = {
      STALE_EVENT: {
        version: 1,
        type: 'event',
        owner: 'demo',
        schema: { type: 'object', properties: {} }
      }
    };
    writeFileSync(path.join(sourceDir, 'stale.json'), JSON.stringify(contract, null, 2));
    runScript(['--source', sourceDir, '--target', targetDir], root);

    // Corrupt the synced file
    writeFileSync(path.join(targetDir, 'stale.json'), '{}');
    const check = runScript(['--source', sourceDir, '--target', targetDir, '--check'], root);
    expect(check.status).toBe(1);
  });

  function createFixture() {
    const root = mkdtempSync(path.join(os.tmpdir(), 'contract-sync-'));
    tmpDirs.push(root);
    const sourceDir = path.join(root, 'shared');
    const targetDir = path.join(root, 'target');
    mkdirSync(sourceDir, { recursive: true });
    mkdirSync(targetDir, { recursive: true });
    return { root, sourceDir, targetDir };
  }
});

function runScript(args, cwd) {
  return spawnSync('node', [SCRIPT_PATH, ...args], {
    cwd,
    encoding: 'utf-8'
  });
}

