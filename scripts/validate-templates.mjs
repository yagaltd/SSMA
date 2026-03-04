import fs from 'node:fs';
import path from 'node:path';

const repoRoot = process.cwd();
const templatesDir = path.join(repoRoot, 'templates');

function fail(message) {
  console.error(`[validate:templates] ${message}`);
  process.exitCode = 1;
}

if (!fs.existsSync(templatesDir)) {
  fail('templates/ directory not found');
  process.exit(process.exitCode || 1);
}

const templates = fs.readdirSync(templatesDir, { withFileTypes: true })
  .filter((entry) => entry.isDirectory())
  .map((entry) => entry.name)
  .sort();

if (templates.length === 0) {
  fail('no template directories found in templates/');
  process.exit(process.exitCode || 1);
}

for (const templateName of templates) {
  const manifestPath = path.join(templatesDir, templateName, 'template.manifest.json');
  if (!fs.existsSync(manifestPath)) {
    fail(`${templateName}: missing template.manifest.json`);
    continue;
  }

  let manifest;
  try {
    manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
  } catch (error) {
    fail(`${templateName}: invalid JSON (${error.message})`);
    continue;
  }

  const requiredKeys = ['templateId', 'engine', 'runtime', 'version', 'schemaVersion', 'requiredFiles'];
  for (const key of requiredKeys) {
    if (!(key in manifest)) {
      fail(`${templateName}: missing required key "${key}"`);
    }
  }

  if (!Array.isArray(manifest.requiredFiles) || manifest.requiredFiles.length === 0) {
    fail(`${templateName}: requiredFiles must be a non-empty array`);
  } else {
    for (const rel of manifest.requiredFiles) {
      const target = path.join(repoRoot, rel);
      if (!fs.existsSync(target)) {
        fail(`${templateName}: required file does not exist: ${rel}`);
      }
    }
  }

  if (manifest.sourcePaths) {
    if (!Array.isArray(manifest.sourcePaths) || manifest.sourcePaths.length === 0) {
      fail(`${templateName}: sourcePaths must be a non-empty array when provided`);
    } else {
      for (const rel of manifest.sourcePaths) {
        const target = path.join(repoRoot, rel);
        if (!fs.existsSync(target)) {
          fail(`${templateName}: source path does not exist: ${rel}`);
        }
      }
    }
  }
}

if (process.exitCode) {
  process.exit(process.exitCode);
}

console.log(`[validate:templates] OK (${templates.length} templates)`);
