'use strict';

const path = require('node:path');

const repo = 'kalamdb/KalamDB';
const artifactPrefix = 'kalamcli';

const supportedPlatforms = Object.freeze([
  { name: 'linux-x86_64', extension: 'tar.gz' },
  { name: 'linux-aarch64', extension: 'tar.gz' },
  { name: 'macos-aarch64', extension: 'tar.gz' },
  { name: 'windows-x86_64', extension: 'zip' },
]);

const supportedPlatformNames = supportedPlatforms.map((platform) => platform.name);

function packageRootFromEnv() {
  return process.env.KALAM_CLI_PACKAGE_ROOT
    ? path.resolve(process.env.KALAM_CLI_PACKAGE_ROOT)
    : path.resolve(__dirname, '..');
}

function installedBinaryPath(packageRoot = packageRootFromEnv()) {
  const installedName = process.platform === 'win32' ? 'kalam.exe' : 'kalam';
  return path.join(packageRoot, 'dist', installedName);
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
  const override = process.env.KALAM_CLI_RELEASE_BASE_URL?.trim();
  if (override) {
    return override.replace(/\/+$/, '');
  }

  return `https://github.com/${repo}/releases/download/v${version}`;
}

module.exports = {
  artifactPrefix,
  archiveNameForPlatform,
  detectPlatform,
  installedBinaryPath,
  packageRootFromEnv,
  releaseBaseUrlForVersion,
  repo,
  supportedPlatformNames,
  supportedPlatforms,
};
