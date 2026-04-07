#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const defaultRoot = path.resolve(__dirname, '..');

const args = process.argv.slice(2);
const options = parseArgs(args);

const rootDir = options.root ? path.resolve(options.root) : defaultRoot;
const sourceDir = options.source
  ? path.resolve(options.source)
  : path.resolve(rootDir, '../../packages/ssma-protocol/contracts');
const targetDir = options.target
  ? path.resolve(options.target)
  : path.join(rootDir, 'src', 'contracts');

if (!fs.existsSync(sourceDir)) {
  console.error(`[sync-contracts] source directory not found: ${sourceDir}`);
  process.exit(1);
}

fs.mkdirSync(targetDir, { recursive: true });

const files = fs.readdirSync(sourceDir).filter((file) => file.endsWith('.json'));
let mismatches = 0;

for (const file of files) {
  const baseName = path.basename(file, '.json');
  const sourcePath = path.join(sourceDir, file);
  const targetJson = path.join(targetDir, file);
  const targetModule = path.join(targetDir, `${baseName}.js`);
  const targetTypes = path.join(targetDir, `${baseName}.d.ts`);

  const jsonContents = fs.readFileSync(sourcePath, 'utf-8');
  const data = JSON.parse(jsonContents);
  const moduleContents = createModuleContents(baseName, jsonContents);
  const dtsContents = createDtsContents(baseName, data);

  mismatches += syncFile(targetJson, jsonContents, options.check);
  mismatches += syncFile(targetModule, moduleContents, options.check);
  mismatches += syncFile(targetTypes, dtsContents, options.check);

  if (!options.check) {
    console.log(`[sync-contracts] synced ${file}`);
  }
}

if (options.check) {
  if (mismatches > 0) {
    console.error(`[sync-contracts] ${mismatches} contract artifact(s) are out of date.`);
    process.exit(1);
  } else {
    console.log('[sync-contracts] contracts are in sync');
  }
}

function syncFile(targetPath, contents, checkOnly) {
  if (checkOnly) {
    if (!fs.existsSync(targetPath)) {
      console.error(`[sync-contracts] missing ${targetPath}`);
      return 1;
    }
    const existing = fs.readFileSync(targetPath, 'utf-8');
    return existing === contents ? 0 : 1;
  }
  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  fs.writeFileSync(targetPath, contents);
  return 0;
}

function createModuleContents(baseName, jsonContents) {
  const varName = `${camelCase(baseName)}Contracts`;
  return `export const ${varName} = ${jsonContents};\nexport default ${varName};\n`;
}

function createDtsContents(baseName, data) {
  const camelName = `${camelCase(baseName)}Contracts`;
  const mapName = `${pascalCase(baseName)}ContractMap`;
  const unionName = `${pascalCase(baseName)}ContractName`;
  const entries = Object.keys(data || {});
  const union = entries.length ? entries.map((name) => `'${name}'`).join(' | ') : 'never';
  const fields = entries.length
    ? entries.map((name) => `  '${name}': ContractDefinition;`).join('\n')
    : '  // no contracts found\n';
  return `export type JsonSchema = Record<string, unknown>;\nexport interface ContractDefinition {\n  version: number;\n  type: string;\n  owner: string;\n  schema: JsonSchema;\n}\nexport type ${unionName} = ${union};\nexport interface ${mapName} {\n${fields}\n}\ndeclare const ${camelName}: ${mapName};\nexport { ${camelName} };\nexport default ${camelName};\n`;
}

function parseArgs(argv) {
  const opts = { check: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    switch (arg) {
      case '--check':
        opts.check = true;
        break;
      case '--root':
        opts.root = argv[i + 1];
        i += 1;
        break;
      case '--source':
        opts.source = argv[i + 1];
        i += 1;
        break;
      case '--target':
        opts.target = argv[i + 1];
        i += 1;
        break;
      case '--help':
        printHelp();
        process.exit(0);
      default:
        console.warn(`[sync-contracts] unknown option: ${arg}`);
    }
  }
  return opts;
}

function printHelp() {
  console.log(`Usage: node sync-contracts.mjs [options]\n\nOptions:\n  --check          Verify existing files without writing\n  --root <path>    Override project root (defaults to repo root)\n  --source <path>  Override shared contracts directory\n  --target <path>  Override target contracts directory`);
}

function camelCase(name) {
  return name
    .replace(/[-_]+([a-zA-Z0-9])/g, (_, c) => c.toUpperCase())
    .replace(/^[A-Z]/, (c) => c.toLowerCase());
}

function pascalCase(name) {
  return name
    .split(/[^a-zA-Z0-9]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join('');
}
