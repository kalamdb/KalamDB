#!/usr/bin/env node
'use strict';

const AdmZip = require('adm-zip');
const crypto = require('node:crypto');
const fs = require('node:fs');
const http = require('node:http');
const https = require('node:https');
const os = require('node:os');
const path = require('node:path');
const tar = require('tar');
const { spawnSync } = require('node:child_process');

const repo = 'kalamdb/KalamDB';
const artifactPrefix = 'kalamcli';
const supportedPlatforms = Object.freeze([
  { name: 'linux-x86_64', extension: 'tar.gz' },
  { name: 'linux-aarch64', extension: 'tar.gz' },
  { name: 'macos-aarch64', extension: 'tar.gz' },
  { name: 'windows-x86_64', extension: 'zip' },
]);
const supportedPlatformNames = supportedPlatforms.map((platform) => platform.name);
const releaseBaseUrlOverride = process.env.KALAM_CLI_RELEASE_BASE_URL?.trim();
const packageRoot = process.env.KALAM_CLI_PACKAGE_ROOT
  ? path.resolve(process.env.KALAM_CLI_PACKAGE_ROOT)
  : path.resolve(__dirname, '..');
const packageJson = require(path.join(packageRoot, 'package.json'));

async function main() {
  if (process.env.KALAM_SKIP_DOWNLOAD === '1') {
    console.log('Skipping KalamDB CLI binary download because KALAM_SKIP_DOWNLOAD=1');
    return;
  }

  const version = (process.env.KALAM_CLI_VERSION || packageJson.version).replace(/^v/, '');
  const platform = detectPlatform();
  const installedBinary = installedBinaryPath();
  const installedVersion = readInstalledBinaryVersion(installedBinary);
  if (installedVersion === version) {
    console.log(`Reusing KalamDB CLI ${version} from ${installedBinary}`);
    logLocalInstallHint(installedBinary);
    return;
  }

  const archiveName = archiveNameForPlatform(version, platform);
  const extension = archiveName.endsWith('.zip') ? 'zip' : 'tar.gz';
  const baseUrl = releaseBaseUrlForVersion(version);
  const archiveUrl = `${baseUrl}/${archiveName}`;
  const checksumsUrl = `${baseUrl}/SHA256SUMS`;
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'kalam-cli-'));

  try {
    console.log(`Downloading ${archiveName}`);
    const archivePath = path.join(tempDir, archiveName);
    const checksumsPath = path.join(tempDir, 'SHA256SUMS');
    await downloadFile(archiveUrl, archivePath);
    await downloadFile(checksumsUrl, checksumsPath);
    verifyChecksum(archivePath, archiveName, checksumsPath);

    const extractDir = path.join(tempDir, 'extract');
    fs.mkdirSync(extractDir, { recursive: true });
    if (extension === 'zip') {
      new AdmZip(archivePath).extractAllTo(extractDir, true);
    } else {
      await tar.x({ file: archivePath, cwd: extractDir });
    }

    const binaryPath = findBinary(extractDir);
    if (!binaryPath) {
      throw new Error(`Could not find kalam binary in ${archiveName}`);
    }

    const distDir = path.join(packageRoot, 'dist');
    fs.mkdirSync(distDir, { recursive: true });
    const installedPath = installedBinary;
    fs.copyFileSync(binaryPath, installedPath);
    if (process.platform !== 'win32') {
      fs.chmodSync(installedPath, 0o755);
    }

    console.log(`Installed KalamDB CLI ${version} to ${installedPath}`);
    logLocalInstallHint(installedPath);
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

function installedBinaryPath() {
  const installedName = process.platform === 'win32' ? 'kalam.exe' : 'kalam';
  return path.join(packageRoot, 'dist', installedName);
}

function readInstalledBinaryVersion(binaryPath) {
  if (!fs.existsSync(binaryPath)) {
    return '';
  }

  const result = spawnSync(binaryPath, ['--version'], {
    encoding: 'utf8',
    timeout: 10000,
    windowsHide: true,
  });
  if (result.error || result.status !== 0) {
    return '';
  }

  const combinedOutput = `${result.stdout || ''}\n${result.stderr || ''}`;
  const match = combinedOutput.match(/^kalam\s+([^\s]+)/m);
  return match?.[1] || '';
}

function logLocalInstallHint(installedPath) {
  if (!isGlobalNpmInstall()) {
    console.log(
      `Installed binary path: ${installedPath}. This was a local npm install, so another kalam already on PATH is unchanged.`
    );
  }
}

function isGlobalNpmInstall() {
  return process.env.npm_config_global === 'true' || process.env.npm_config_global === '1';
}

function detectPlatform() {
  const osName = (() => {
    switch (process.platform) {
      case 'linux':
        return 'linux';
      case 'darwin':
        return 'macos';
      case 'win32':
        return 'windows';
      default:
        throw new Error(`Unsupported operating system: ${process.platform}`);
    }
  })();

  const arch = (() => {
    switch (process.arch) {
      case 'x64':
        return 'x86_64';
      case 'arm64':
        return 'aarch64';
      default:
        throw new Error(`Unsupported architecture: ${process.arch}`);
    }
  })();

  const platform = `${osName}-${arch}`;
  if (!supportedPlatforms.some((candidate) => candidate.name === platform)) {
    throw new Error(
      `Unsupported platform: ${platform}. Supported platforms: ${supportedPlatformNames.join(', ')}`
    );
  }

  return platform;
}

function archiveNameForPlatform(version, platform) {
  const supported = supportedPlatforms.find((candidate) => candidate.name === platform);
  if (!supported) {
    throw new Error(
      `Unsupported platform: ${platform}. Supported platforms: ${supportedPlatformNames.join(', ')}`
    );
  }

  return `${artifactPrefix}-${version}-${platform}.${supported.extension}`;
}

function releaseBaseUrlForVersion(version) {
  return releaseBaseUrlOverride
    ? releaseBaseUrlOverride.replace(/\/+$/, '')
    : `https://github.com/${repo}/releases/download/v${version}`;
}

function downloadFile(url, destination) {
  return new Promise((resolve, reject) => {
    const client = url.startsWith('http://') ? http : https;
    const request = client.get(
      url,
      {
        headers: {
          'User-Agent': `@kalamdb/cli/${packageJson.version}`,
        },
      },
      (response) => {
        if ([301, 302, 303, 307, 308].includes(response.statusCode)) {
          response.resume();
          const location = response.headers.location;
          if (!location) {
            reject(new Error(`Redirect without location for ${url}`));
            return;
          }
          downloadFile(new URL(location, url).toString(), destination).then(resolve, reject);
          return;
        }

        if (response.statusCode !== 200) {
          response.resume();
          reject(new Error(`Download failed for ${url} with HTTP ${response.statusCode}`));
          return;
        }

        const file = fs.createWriteStream(destination, { mode: 0o600 });
        response.pipe(file);
        file.on('finish', () => file.close(resolve));
        file.on('error', reject);
      }
    );

    request.on('error', reject);
    request.setTimeout(120000, () => {
      request.destroy(new Error(`Timed out downloading ${url}`));
    });
  });
}

function verifyChecksum(archivePath, archiveName, checksumsPath) {
  const checksums = fs.readFileSync(checksumsPath, 'utf8');
  const expected = checksums
    .split(/\r?\n/)
    .map((line) => line.trim().split(/\s+/))
    .find((parts) => parts.length >= 2 && normalizeChecksumName(parts[1]) === archiveName)?.[0];

  if (!expected) {
    throw new Error(`SHA256SUMS does not include ${archiveName}`);
  }

  const actual = crypto.createHash('sha256').update(fs.readFileSync(archivePath)).digest('hex');
  if (actual !== expected) {
    throw new Error(`Checksum mismatch for ${archiveName}`);
  }
}

function normalizeChecksumName(name) {
  return name.replace(/^\*/, '').replace(/^\.\//, '');
}

function findBinary(root) {
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const entryPath = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(entryPath);
      } else if (
        ['kalam', 'kalam.exe', artifactPrefix].includes(entry.name) ||
        entry.name.startsWith(`${artifactPrefix}-`)
      ) {
        return entryPath;
      }
    }
  }
  return null;
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`Failed to install KalamDB CLI: ${error.message}`);
    process.exit(1);
  });
}

module.exports = {
  archiveNameForPlatform,
  detectPlatform,
  downloadFile,
  findBinary,
  installedBinaryPath,
  isGlobalNpmInstall,
  logLocalInstallHint,
  readInstalledBinaryVersion,
  releaseBaseUrlForVersion,
  supportedPlatforms,
  verifyChecksum,
};