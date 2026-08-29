import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const packageDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const { quoteCmdPath, replaceFile, unlockInstalledBinary } = require('../scripts/replace-binary.js');

test('replaceFile overwrites an existing destination', (t) => {
  const root = mkdtempSync(path.join(tmpdir(), 'kalam-replace-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));

  const source = path.join(root, 'incoming');
  const destination = path.join(root, 'dist', 'kalam');
  mkdirSync(path.dirname(destination), { recursive: true });
  writeFileSync(destination, 'old-binary');
  writeFileSync(source, 'new-binary');

  replaceFile(source, destination);

  assert.equal(readFileSync(destination, 'utf8'), 'new-binary');
  assert.equal(readFileSync(source, 'utf8'), 'new-binary');
});

test('unlockInstalledBinary moves kalam.exe out of the package directory', (t) => {
  const root = mkdtempSync(path.join(tmpdir(), 'kalam-unlock-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));

  const distDir = path.join(root, 'dist');
  mkdirSync(distDir, { recursive: true });
  const binaryPath = path.join(distDir, process.platform === 'win32' ? 'kalam.exe' : 'kalam');
  writeFileSync(binaryPath, 'installed-binary');

  const unlockedPath = unlockInstalledBinary(root);
  t.after(() => {
    if (unlockedPath) {
      rmSync(unlockedPath, { force: true });
    }
  });

  assert.ok(unlockedPath, 'expected unlock to return the moved path');
  assert.equal(readFileSync(unlockedPath, 'utf8'), 'installed-binary');
  assert.equal(
    require('node:fs').existsSync(binaryPath),
    false,
    'package dist binary should be gone so npm can delete the directory',
  );
});

test('quoteCmdPath wraps Windows paths and escapes quotes', () => {
  assert.equal(quoteCmdPath(String.raw`C:\Users\O"Brien\kalam.exe`), String.raw`"C:\Users\O""Brien\kalam.exe"`);
});
