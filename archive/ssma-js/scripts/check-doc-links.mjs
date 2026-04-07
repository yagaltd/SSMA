import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const root = path.resolve(__dirname, '../../..');
const docsDir = path.join(root, 'docs');

function listMarkdownFiles(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      out.push(...listMarkdownFiles(full));
      continue;
    }
    if (entry.isFile() && entry.name.endsWith('.md')) {
      out.push(full);
    }
  }
  return out;
}

function extractLinks(markdown) {
  const links = [];
  const regex = /\[[^\]]*\]\(([^)]+)\)/g;
  let match;
  while ((match = regex.exec(markdown)) !== null) {
    links.push(match[1]);
  }
  return links;
}

const mdFiles = listMarkdownFiles(docsDir);
const missing = [];

for (const file of mdFiles) {
  const raw = fs.readFileSync(file, 'utf8');
  const links = extractLinks(raw);
  for (const link of links) {
    if (!link || link.startsWith('http://') || link.startsWith('https://') || link.startsWith('#')) {
      continue;
    }
    const clean = link.split('#')[0].split('?')[0];
    if (!clean) continue;
    const resolved = path.resolve(path.dirname(file), clean);
    if (!fs.existsSync(resolved)) {
      missing.push({ file: path.relative(root, file), link });
    }
  }
}

if (missing.length) {
  console.error('[docs] Missing link targets:');
  for (const hit of missing) {
    console.error(`- ${hit.file}: ${hit.link}`);
  }
  process.exit(1);
}

console.log(`[docs] link check passed (${mdFiles.length} markdown files)`);
