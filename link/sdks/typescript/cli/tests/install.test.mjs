import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import http from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import * as tar from 'tar';

const packageDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const packageJsonPath = path.join(packageDir, 'package.json');
const installScriptPath = path.join(packageDir, 'scripts', 'install.js');
const fixtureVersion = '9.9.9-test.1';
const fixtureOutput = 'npm-cli-installed-fixture';

test('install script downloads and installs the wrapped kalam binary', async (t) => {
  if (process.platform === 'win32') {
    t.skip('unix shell fixture only');
  }

  const installRoot = mkdtempSync(path.join(tmpdir(), 'kalam-npm-install-'));
  t.after(() => rmSync(installRoot, { recursive: true, force: true }));
  cpSync(packageJsonPath, path.join(installRoot, 'package.json'));

  const platform = detectPlatform();
  const archiveName = `kalamcli-${fixtureVersion}-${platform}.tar.gz`;
  const releaseDir = mkdtempSync(path.join(tmpdir(), 'kalam-npm-release-'));
  t.after(() => rmSync(releaseDir, { recursive: true, force: true }));

  const archivePath = path.join(releaseDir, archiveName);
  await createFixtureArchive(releaseDir, archivePath, fixtureVersion, platform, fixtureOutput);
  const checksum = createHash('sha256').update(readFileSync(archivePath)).digest('hex');
  const checksums = `${checksum}  ${archiveName}\n`;

  const server = http.createServer((req, res) => {
    if (req.url === `/releases/download/v${fixtureVersion}/${archiveName}`) {
      const body = readFileSync(archivePath);
      res.writeHead(200, { 'Content-Length': body.length });
      res.end(body);
      return;
    }

    if (req.url === `/releases/download/v${fixtureVersion}/SHA256SUMS`) {
      res.writeHead(200, { 'Content-Length': Buffer.byteLength(checksums) });
      res.end(checksums);
      return;
    }

    res.writeHead(404);
    res.end('not found');
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(() => server.close());

  const address = server.address();
  assert(address && typeof address === 'object');
  const baseUrl = `http://127.0.0.1:${address.port}/releases/download/v${fixtureVersion}`;

  const result = await runProcess(process.execPath, [installScriptPath], {
    cwd: packageDir,
    encoding: 'utf8',
    env: {
      ...process.env,
      KALAM_CLI_PACKAGE_ROOT: installRoot,
      KALAM_CLI_RELEASE_BASE_URL: baseUrl,
      KALAM_CLI_VERSION: fixtureVersion,
      NO_PROXY: '127.0.0.1,localhost,::1',
      no_proxy: '127.0.0.1,localhost,::1',
    },
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);

  const installedBinary = path.join(installRoot, 'dist', process.platform === 'win32' ? 'kalam.exe' : 'kalam');
  assert.ok(existsSync(installedBinary), 'expected install script to place a binary in dist/');

  const binaryRun = spawnSync(installedBinary, [], { encoding: 'utf8' });
  assert.equal(binaryRun.status, 0, binaryRun.stderr || binaryRun.stdout);
  assert.equal(binaryRun.stdout.trim(), fixtureOutput);
});

test('install script reuses the existing dist binary when the version already matches', async (t) => {
  if (process.platform === 'win32') {
    t.skip('unix shell fixture only');
  }

  const installRoot = mkdtempSync(path.join(tmpdir(), 'kalam-npm-reuse-'));
  t.after(() => rmSync(installRoot, { recursive: true, force: true }));
  cpSync(packageJsonPath, path.join(installRoot, 'package.json'));
  mkdirSync(path.join(installRoot, 'dist'), { recursive: true });

  const binaryPath = path.join(installRoot, 'dist', 'kalam');
  writeFileSync(
    binaryPath,
    `#!/bin/sh
if [ "$1" = "--version" ]; then
  printf 'kalam ${fixtureVersion}\\n'
else
  printf 'reused-binary\\n'
fi
`
  );
  chmodSync(binaryPath, 0o755);

  const result = await runProcess(process.execPath, [installScriptPath], {
    cwd: packageDir,
    encoding: 'utf8',
    env: {
      ...process.env,
      KALAM_CLI_PACKAGE_ROOT: installRoot,
      KALAM_CLI_VERSION: fixtureVersion,
      KALAM_CLI_RELEASE_BASE_URL: 'http://127.0.0.1:9/releases/download/v9.9.9-test.1',
      NO_PROXY: '127.0.0.1,localhost,::1',
      no_proxy: '127.0.0.1,localhost,::1',
    },
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.match(result.stdout, /Reusing KalamDB CLI 9\.9\.9-test\.1/);
  assert.doesNotMatch(result.stdout, /Downloading kalamcli-/);

  const binaryRun = spawnSync(binaryPath, [], { encoding: 'utf8' });
  assert.equal(binaryRun.status, 0, binaryRun.stderr || binaryRun.stdout);
  assert.equal(binaryRun.stdout.trim(), 'reused-binary');
});

function runProcess(command, args, options) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      ...options,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '';
    let stderr = '';

    child.stdout.setEncoding('utf8');
    child.stderr.setEncoding('utf8');
    child.stdout.on('data', (chunk) => {
      stdout += chunk;
    });
    child.stderr.on('data', (chunk) => {
      stderr += chunk;
    });
    child.on('error', reject);
    child.on('close', (code, signal) => {
      resolve({ status: code, signal, stdout, stderr });
    });
  });
}

function detectPlatform() {
  const osName = process.platform === 'darwin' ? 'macos' : process.platform;
  const arch = process.arch === 'x64' ? 'x86_64' : process.arch === 'arm64' ? 'aarch64' : process.arch;
  return `${osName}-${arch}`;
}

async function createFixtureArchive(releaseDir, archivePath, version, platform, output) {
  const stagingRoot = path.join(releaseDir, `kalamcli-${version}-${platform}`);
  const binaryPath = path.join(stagingRoot, 'kalam');
  mkdirSync(stagingRoot, { recursive: true });
  writeFileSync(binaryPath, `#!/bin/sh\nprintf '%s\\n' '${output}'\n`);
  chmodSync(binaryPath, 0o755);
  await tar.c({ cwd: releaseDir, file: archivePath, gzip: true }, [path.basename(stagingRoot)]);
}