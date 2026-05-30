#!/usr/bin/env node
'use strict';

const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');

const packageRoot = process.env.KALAM_CLI_PACKAGE_ROOT
  ? path.resolve(process.env.KALAM_CLI_PACKAGE_ROOT)
  : path.resolve(__dirname, '..');
const binaryName = process.platform === 'win32' ? 'kalam.exe' : 'kalam';
const binaryPath = path.resolve(packageRoot, 'dist', binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error('KalamDB CLI binary was not installed. Try reinstalling @kalamdb/cli.');
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status === null ? 1 : result.status);