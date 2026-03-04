#!/usr/bin/env node
import fs from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const projectRoot = path.resolve(__dirname, '..');

async function scanServices() {
  const servicesDir = path.join(projectRoot, 'src', 'services');
  const services = [];

  try {
    const entries = await fs.readdir(servicesDir, { withFileTypes: true });
    for (const entry of entries) {
      if (!entry.isDirectory()) continue;
      const servicePath = path.join(servicesDir, entry.name);
      const files = await fs.readdir(servicePath);
      services.push({
        name: entry.name,
        files,
        routes: files.filter((file) => file.includes('routes')).length,
        controllers: files.filter((file) => file.includes('controller')).length
      });
    }
  } catch (error) {
    return [];
  }

  return services;
}

async function readContracts() {
  const contractsDir = path.join(projectRoot, 'src', 'contracts');
  try {
    const files = await fs.readdir(contractsDir);
    return files.filter((file) => file.endsWith('.json'));
  } catch (error) {
    return [];
  }
}

async function getPackageInfo() {
  try {
    const pkg = await fs.readFile(path.join(projectRoot, 'package.json'), 'utf-8');
    return JSON.parse(pkg);
  } catch (error) {
    return {};
  }
}

async function generateMap() {
  const services = await scanServices();
  const contracts = await readContracts();
  const pkg = await getPackageInfo();

  const map = {
    generated: new Date().toISOString(),
    project: {
      name: pkg.name,
      version: pkg.version,
      description: pkg.description
    },
    architecture: {
      pattern: 'SSMA',
      portabilityTargets: ['Node', 'Bun', 'Cloudflare Workers', 'MV3'],
      eventBus: true
    },
    services,
    contracts,
    scripts: Object.keys(pkg.scripts || {}),
    dependencies: pkg.dependencies || {}
  };

  const output = path.join(projectRoot, 'ssma-system-map.json');
  await fs.writeFile(output, JSON.stringify(map, null, 2));
  console.log(`[generate-ssma-map] wrote ${output}`);
}

generateMap().catch((error) => {
  console.error('[generate-ssma-map] failed', error);
  process.exit(1);
});
