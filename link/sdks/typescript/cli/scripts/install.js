#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const { bootstrapBinary } = require('./bootstrap');
const { tryInstallFromLocalBinary } = require('./local-binary');
const { installedBinaryPath, packageRootFromEnv } = require('./platforms');

async function main() {
  if (process.env.KALAM_SKIP_DOWNLOAD === '1') {
    console.log('Skipping KalamDB CLI binary download because KALAM_SKIP_DOWNLOAD=1');
    return;
  }

  const packageRoot = packageRootFromEnv();
  const packageJson = require(path.join(packageRoot, 'package.json'));
  const version = (process.env.KALAM_CLI_VERSION || packageJson.version).replace(/^v/, '');
  const binaryPath = installedBinaryPath(packageRoot);
  const userAgent = `@kalamdb/cli/${packageJson.version}`;

  if (!fs.existsSync(binaryPath) && !tryInstallFromLocalBinary(packageRoot)) {
    await bootstrapBinary(packageRoot, version, userAgent);
  }

  runKalamUpdate(binaryPath, version);
  logLocalInstallHint(binaryPath);
}

function runKalamUpdate(binaryPath, version) {
  const result = spawnSync(
    binaryPath,
    ['--no-color', '--no-spinner', 'update', '--version', version],
    {
      stdio: 'inherit',
      env: process.env,
    }
  );

  if (result.error) {
    throw result.error;
  }

  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
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

if (require.main === module) {
  main().catch((error) => {
    console.error(`Failed to install KalamDB CLI: ${error.message}`);
    process.exit(1);
  });
}

module.exports = {
  installedBinaryPath,
  isGlobalNpmInstall,
  logLocalInstallHint,
  runKalamUpdate,
};
