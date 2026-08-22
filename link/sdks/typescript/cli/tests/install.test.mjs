import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from 'node:fs';
import http from 'node:http';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import * as tar from 'tar';

const require = createRequire(import.meta.url);

const packageDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const packageJsonPath = path.join(packageDir, 'package.json');
const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
const installScriptPath = path.join(packageDir, 'scripts', 'install.js');
const fixtureVersion = packageJson.version;

function resolveKalamBinaryForTests() {
  const { localKalamBinaryCandidates } = require('../scripts/local-binary.js');
  const { installedBinaryPath } = require('../scripts/platforms.js');
  const installed = installedBinaryPath(packageDir);

  return (
    localKalamBinaryCandidates(packageDir).find(
      (candidate) => candidate !== installed && existsSync(candidate),
    ) ?? null
  );
}

test('install script bootstraps then delegates verification to kalam update', async (t) => {
  if (process.platform === 'win32') {
    t.skip('unix fixture only');
  }

  const kalamBinary = resolveKalamBinaryForTests();
  if (!kalamBinary) {
    t.skip('kalam binary not available for install test');
  }

  const installRoot = mkdtempSync(path.join(tmpdir(), 'kalam-npm-install-'));
  t.after(() => rmSync(installRoot, { recursive: true, force: true }));
  cpSync(packageJsonPath, path.join(installRoot, 'package.json'));

  const platform = detectPlatform();
  const archiveName = `kalamcli-${fixtureVersion}-${platform}.tar.gz`;
  const releaseDir = mkdtempSync(path.join(tmpdir(), 'kalam-npm-release-'));
  t.after(() => rmSync(releaseDir, { recursive: true, force: true }));

  const archivePath = path.join(releaseDir, archiveName);
  await createFixtureArchive(releaseDir, archivePath, fixtureVersion, platform, kalamBinary);
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
      KALAM_SKIP_MANAGED_SERVER_UPDATE: '1',
      NO_PROXY: '127.0.0.1,localhost,::1',
      no_proxy: '127.0.0.1,localhost,::1',
    },
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.match(result.stdout, /Bootstrapping kalamcli-/);

  const installedBinary = path.join(installRoot, 'dist', 'kalam');
  assert.ok(existsSync(installedBinary), 'expected install script to place a binary in dist/');

  const versionRun = spawnSync(installedBinary, ['--version'], { encoding: 'utf8' });
  assert.equal(versionRun.status, 0, versionRun.stderr || versionRun.stdout);
  assert.match(versionRun.stdout, /^kalam\s+/m);
});

test('install script delegates to kalam update when a binary already exists', async (t) => {
  if (process.platform === 'win32') {
    t.skip('unix fixture only');
  }

  const kalamBinary = resolveKalamBinaryForTests();
  if (!kalamBinary) {
    t.skip('kalam binary not available for install test');
  }

  const installRoot = mkdtempSync(path.join(tmpdir(), 'kalam-npm-reuse-'));
  t.after(() => rmSync(installRoot, { recursive: true, force: true }));
  cpSync(packageJsonPath, path.join(installRoot, 'package.json'));
  mkdirSync(path.join(installRoot, 'dist'), { recursive: true });

  const binaryPath = path.join(installRoot, 'dist', 'kalam');
  copyFileSync(kalamBinary, binaryPath);
  chmodSync(binaryPath, 0o755);

  const platform = detectPlatform();
  const archiveName = `kalamcli-${fixtureVersion}-${platform}.tar.gz`;
  const releaseDir = mkdtempSync(path.join(tmpdir(), 'kalam-npm-release-'));
  t.after(() => rmSync(releaseDir, { recursive: true, force: true }));

  const archivePath = path.join(releaseDir, archiveName);
  await createFixtureArchive(releaseDir, archivePath, fixtureVersion, platform, kalamBinary);
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
      KALAM_CLI_VERSION: fixtureVersion,
      KALAM_CLI_RELEASE_BASE_URL: baseUrl,
      KALAM_SKIP_MANAGED_SERVER_UPDATE: '1',
      NO_PROXY: '127.0.0.1,localhost,::1',
      no_proxy: '127.0.0.1,localhost,::1',
    },
  });

  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.doesNotMatch(result.stdout, /Bootstrapping kalamcli-/);
  assert.ok(existsSync(binaryPath), 'expected existing binary to remain installed');
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

async function createFixtureArchive(releaseDir, archivePath, version, platform, kalamBinary) {
  const entryName = `kalamcli-${version}-${platform}`;
  const binaryPath = path.join(releaseDir, entryName);
  copyFileSync(kalamBinary, binaryPath);
  chmodSync(binaryPath, 0o755);
  await tar.c({ cwd: releaseDir, file: archivePath, gzip: true }, [entryName]);
}
