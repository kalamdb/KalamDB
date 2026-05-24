import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const packageDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const launcherPath = path.join(packageDir, 'bin', 'kalam.js');

test('launcher executes the installed kalam binary from dist/', (t) => {
  if (process.platform === 'win32') {
    t.skip('unix shell fixture only');
  }

  const installRoot = mkdtempSync(path.join(tmpdir(), 'kalam-npm-launcher-'));
  t.after(() => rmSync(installRoot, { recursive: true, force: true }));
  mkdirSync(path.join(installRoot, 'dist'), { recursive: true });

  const binaryPath = path.join(installRoot, 'dist', 'kalam');
  writeFileSync(binaryPath, "#!/bin/sh\nprintf 'launcher:%s\\n' \"$1\"\n");
  chmodSync(binaryPath, 0o755);

  const result = spawnSync(process.execPath, [launcherPath, 'doctor'], {
    cwd: packageDir,
    encoding: 'utf8',
    env: {
      ...process.env,
      KALAM_CLI_PACKAGE_ROOT: installRoot,
    },
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.equal(result.stdout.trim(), 'launcher:doctor');
});