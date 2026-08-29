'use strict';

const fs = require('node:fs');
const path = require('node:path');

const { installedBinaryPath } = require('./platforms');
const { replaceFile } = require('./replace-binary');

function monorepoRootFromPackageRoot(packageRoot) {
  return path.resolve(packageRoot, '../../../../');
}

function localKalamBinaryCandidates(packageRoot) {
  const binaryName = process.platform === 'win32' ? 'kalam.exe' : 'kalam';
  const repoRoot = monorepoRootFromPackageRoot(packageRoot);

  return [
    process.env.KALAM_TEST_BINARY,
    process.env.KALAM_BIN,
    path.join(packageRoot, 'dist', binaryName),
    path.join(repoRoot, 'target', 'debug', binaryName),
    path.join(repoRoot, 'target', 'release', binaryName),
  ].filter(Boolean);
}

function shouldUseLocalBinaryFallback() {
  if (process.env.KALAM_SKIP_LOCAL_BINARY === '1') {
    return false;
  }

  if (process.env.KALAM_CLI_RELEASE_BASE_URL?.trim()) {
    return false;
  }

  return true;
}

function tryInstallFromLocalBinary(packageRoot) {
  if (!shouldUseLocalBinaryFallback()) {
    return false;
  }

  const installedPath = installedBinaryPath(packageRoot);
  if (fs.existsSync(installedPath)) {
    return false;
  }

  const source = localKalamBinaryCandidates(packageRoot).find((candidate) => {
    return candidate !== installedPath && fs.existsSync(candidate);
  });
  if (!source) {
    return false;
  }

  const distDir = path.join(packageRoot, 'dist');
  fs.mkdirSync(distDir, { recursive: true });
  replaceFile(source, installedPath);

  console.log(`Using local KalamDB CLI binary from ${source}`);
  return true;
}

function hasWorkspaceKalamBinary(packageRoot) {
  if (!shouldUseLocalBinaryFallback()) {
    return false;
  }

  const installedPath = installedBinaryPath(packageRoot);
  return localKalamBinaryCandidates(packageRoot).some(
    (candidate) => candidate !== installedPath && fs.existsSync(candidate),
  );
}

module.exports = {
  hasWorkspaceKalamBinary,
  localKalamBinaryCandidates,
  monorepoRootFromPackageRoot,
  shouldUseLocalBinaryFallback,
  tryInstallFromLocalBinary,
};
